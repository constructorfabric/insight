"""Per-fixture identity namespacing for seed-once execution.

In seed-once mode EVERY fixture's bronze rows coexist in one ClickHouse at the
same time. Two problems follow from that, and this module solves both by giving
each fixture a private identity namespace derived from its name (the `token`):

  * ReplacingMergeTree collapse — bronze tables are keyed by `email`-family
    columns or by `unique_key`. If two fixtures both seed `alice@example.com`
    (or `unique_key: bamboohr-test-e001`), RMT merges them into ONE row and one
    fixture's data is silently destroyed. Rewriting identity per fixture makes
    every key unique, so all rows survive.
  * Distribution / join bleed — a bullet's median/p25/p75 aggregates over
    `org_unit_id` (= the person's `department`), and connector joins resolve by
    `(source_id, account_id)`. Sharing those across fixtures pools everyone into
    one department / one join scope. Per-fixture rewrites keep each fixture's
    aggregate and joins self-contained.

The SAME rewrite is applied to seeded bronze records AND to each case's request,
in lockstep: a query for `erin@example.com` in fixture `foo` is rewritten to
`erin@foo.example.com` and matches foo's — and only foo's — seeded rows.

Rewrites (token T):
  1. email domain `@example.com` -> `@T.example.com` on every string value.
       Covers every email-family RMT key (email / user_email / userPrincipalName
       / author_email / email_address / …), the `person_id eq '<email>'` request
       filter, and cross-refs such as `supervisorEmail` — both sides rewrite
       identically so the org-chart edge survives. Also rewrites emails embedded
       in JSON blobs (e.g. jira `custom_fields_json`).
  2. `unique_key`  -> `T-<value>`  — covers every unique_key-keyed RMT table.
  3. `department`  -> `T-<value>`  — isolates the `org_unit_id` distribution;
       the `org_unit_id eq '<dept>'` request filter is rewritten to match.
  4. `source_id`   -> `T-<value>`  — scopes connector joins keyed by
       `(source_id, account_id)` (e.g. jira assignee → email) to one fixture.

The invariant these enforce — no two fixtures share an RMT key or a connector
join key — is asserted by `meta/test_seed_isolation.py`. `bronze_zoom.meetings`
(ORDER BY uuid) is the one RMT table these rules would not isolate; no fixture
seeds it, and the guard test fails loudly if that ever changes.
"""

from __future__ import annotations

import copy
import re

_EMAIL_DOMAIN = "@example.com"

# Fields prefixed with `T-`. These cover the RMT ORDER BY keys not handled by the
# email rewrite — `unique_key` (most bronze tables) and `id` (bamboohr.employees
# and jira_issue are `ORDER BY id`, and `id` is a shared literal like `e001` /
# `TDW-1` across fixtures, so without this their rows collapse on merge) — plus
# the two grouping/join keys that must be per-fixture: `department` (the
# org_unit_id distribution) and `source_id` (connector join scope). The guard
# `meta/test_seed_isolation.py` derives the real ORDER BY key of every seeded
# table from the placeholder DDL and fails if any prefix is missing.
_PREFIXED_FIELDS = ("unique_key", "department", "source_id", "id")

_ORG_UNIT_FILTER = re.compile(r"(org_unit_id\s+eq\s+')([^']*)(')")


def token_for(name: str) -> str:
    """Fixture name -> namespace token. Fixture stems are already `[a-z0-9_]+`,
    which is a safe opaque label inside a CH String (the API matches `person_id`
    by exact string, so DNS-validity of the synthetic domain is irrelevant)."""
    return name


def _domain_rewrite(value: str, token: str) -> str:
    return value.replace(_EMAIL_DOMAIN, f"@{token}.example.com")


def namespace_record(rec: dict, token: str) -> dict:
    """Return a copy of a resolved bronze record with identity rewritten for `token`."""
    out: dict = {}
    for k, v in rec.items():
        if isinstance(v, str) and _EMAIL_DOMAIN in v:
            v = _domain_rewrite(v, token)  # emails anywhere, incl. JSON blobs
        if k in _PREFIXED_FIELDS and isinstance(v, str) and v:
            v = f"{token}-{v}"
        out[k] = v
    return out


def namespace_bronze(bronze: dict[str, list[dict]], token: str) -> dict[str, list[dict]]:
    """Namespace every record of every bronze table for `token`."""
    return {tbl: [namespace_record(r, token) for r in rows] for tbl, rows in bronze.items()}


def namespace_request(request: dict, token: str) -> dict:
    """Return a deep copy of a case request with its `$filter`s rewritten for `token`.

    Requests only ever filter on `person_id` (an email → rule 1) or `org_unit_id`
    (a department → rule 3); nothing else in the body references identity.
    """
    req = copy.deepcopy(request)
    body = req.get("body") or {}
    for q in body.get("queries", []):
        f = q.get("$filter")
        if not isinstance(f, str):
            continue
        f = _domain_rewrite(f, token)  # person_id eq '<email>'
        f = _ORG_UNIT_FILTER.sub(lambda m: f"{m.group(1)}{token}-{m.group(2)}{m.group(3)}", f)
        q["$filter"] = f
    return req
