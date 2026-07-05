"""Guard: prove the seed-once namespacing keeps every fixture's data disjoint.

In seed-once mode all fixtures' bronze coexists in one ClickHouse. Correctness
hinges on `lib.namespace` making each fixture's identity unique so that (a)
ReplacingMergeTree never collapses two fixtures' rows into one, and (b) no query
sees a neighbour's data. This test asserts that invariant WITHOUT a database, so
a namespacing regression (or a new fixture/table that dodges the rules) fails
fast in CI before the stack is ever built.

The load-bearing check derives each bronze table's REAL `ORDER BY` key from the
placeholder DDL (`create-bronze-placeholders.sh` — the same schema the rig
seeds into) and asserts that, after namespacing, no two fixtures share a key
tuple for the same table. That is exactly the RMT merge key, so a pass proves no
cross-fixture collapse is possible. (This is the check that would have caught
`bronze_bamboohr.employees ORDER BY id` / `bronze_jira.jira_issue ORDER BY id`
sharing the literal `id`s `e001` / `TDW-1` across fixtures.)
"""

from __future__ import annotations

import re
from collections import defaultdict
from pathlib import Path

import pytest

from lib import namespace
from lib.fixture_loader import discover_tests, load as load_test

pytestmark = pytest.mark.offline

_E2E_ROOT = Path(__file__).resolve().parent.parent
_METRICS_ROOT = _E2E_ROOT / "metrics"
_PLACEHOLDER_SQL = _E2E_ROOT.parents[1] / "scripts" / "create-bronze-placeholders.sh"
_BARE_DOMAIN = "@example.com"
_PREFIXED = ("unique_key", "source_id", "department", "id")

# CREATE TABLE bronze_x.y ( ... ) ENGINE = ... ORDER BY <key> [SETTINGS|COMMENT|;]
_CREATE_ORDER_BY = re.compile(
    r"CREATE TABLE IF NOT EXISTS\s+(bronze_\w+\.\w+)\b.*?ORDER BY\s+(.+?)(?:\s+SETTINGS|\s+COMMENT|;)",
    re.S,
)


def _order_by_key_columns() -> dict[str, list[str]]:
    """Map `bronze_db.table` -> the column names in its ORDER BY, parsed from the
    placeholder DDL (the schema the rig actually seeds into)."""
    sql = _PLACEHOLDER_SQL.read_text(encoding="utf-8")
    out: dict[str, list[str]] = {}
    for m in _CREATE_ORDER_BY.finditer(sql):
        table = m.group(1)
        raw = m.group(2).strip().strip("()")
        cols = [c.strip() for c in raw.split(",") if c.strip()]
        out[table] = cols
    return out


@pytest.fixture(scope="module")
def namespaced_fixtures():
    """Every fixture, namespaced by its token: list of (name, token, bronze)."""
    out = []
    for path in discover_tests(_METRICS_ROOT):
        ty = load_test(path)
        token = namespace.token_for(ty.name)
        out.append((ty.name, token, namespace.namespace_bronze(ty.bronze, token)))
    assert out, "no fixtures discovered under metrics/"
    return out


@pytest.fixture(scope="module")
def order_by_keys():
    keys = _order_by_key_columns()
    assert keys, f"parsed no ORDER BY keys from {_PLACEHOLDER_SQL}"
    return keys


def _iter_values(bronze):
    for rows in bronze.values():
        for rec in rows:
            for k, v in rec.items():
                yield k, v


def test_tokens_are_unique(namespaced_fixtures):
    tokens = [tok for _n, tok, _b in namespaced_fixtures]
    dupes = {t for t in tokens if tokens.count(t) > 1}
    assert not dupes, f"fixture tokens collide (identity would merge): {sorted(dupes)}"


def test_email_domain_fully_rewritten(namespaced_fixtures):
    """No record value still carries the bare shared domain — the rewrite that
    makes email-keyed RMT tables unique must have hit every string, including
    emails embedded in JSON blobs."""
    leaks = []
    for name, _tok, bronze in namespaced_fixtures:
        for _k, v in _iter_values(bronze):
            if isinstance(v, str) and _BARE_DOMAIN in v:
                leaks.append((name, v))
    assert not leaks, f"un-namespaced `@example.com` survives (RMT collapse risk): {leaks[:10]}"


def test_prefixed_fields_are_token_scoped(namespaced_fixtures):
    bad = []
    for _name, tok, bronze in namespaced_fixtures:
        for k, v in _iter_values(bronze):
            if k in _PREFIXED and isinstance(v, str) and v and not v.startswith(f"{tok}-"):
                bad.append((tok, k, v))
    assert not bad, f"prefixed identity fields not token-scoped: {bad[:10]}"


def test_rmt_order_by_key_disjoint_across_fixtures(namespaced_fixtures, order_by_keys):
    """THE load-bearing check: for every seeded bronze table, no two fixtures may
    share an ORDER BY key tuple after namespacing — that tuple IS the RMT merge
    key, so sharing it means one fixture's rows silently overwrite another's.

    A seeded table whose ORDER BY key is unknown (not in the placeholder DDL) is
    an error: we cannot prove it is collision-free, so fail loudly rather than
    seed blind.
    """
    unknown: set[str] = set()
    owners: dict[tuple[str, tuple], set[str]] = defaultdict(set)
    for name, _tok, bronze in namespaced_fixtures:
        for fqn, rows in bronze.items():
            cols = order_by_keys.get(fqn)
            if cols is None:
                unknown.add(fqn)
                continue
            for rec in rows:
                key = tuple(rec.get(c) for c in cols)
                owners[(fqn, key)].add(name)

    assert not unknown, (
        f"seeded bronze tables with no ORDER BY key in the placeholder DDL "
        f"(cannot prove collision-free): {sorted(unknown)}"
    )
    shared = {k: sorted(v) for k, v in owners.items() if len(v) > 1}
    assert not shared, (
        "ORDER BY key shared across fixtures — RMT would collapse these rows. "
        f"Namespace the offending key column(s) in lib.namespace. Examples: "
        f"{dict(list(shared.items())[:5])}"
    )
