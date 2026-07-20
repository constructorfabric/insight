"""
People / org-linkage seed.

Populates:

* `silver.class_people` — one row per person with `department_name`
  set to the team. The `insight.team_member` view joins on this.
* `bronze_bamboohr.employees` — emails + departments + supervisorEmail
  so `insight.people` can compute `org_unit_id` and the supervisor
  chain from a BambooHR-shaped table. The columns used by the
  view are workEmail, displayName, department, jobTitle, supervisorEmail.

`silver.class_people` lowercases its `email` column so case-insensitive
joins downstream (notably `insight.team_member`, which compares against
`lower(...)`) match cleanly. `bronze_bamboohr.employees` keeps the
original casing that a real BambooHR feed would deliver — fine here
because the seed roster (`profiles.py`) already uses lowercase
addresses end-to-end, so no identities split in practice. If a future
roster introduces mixed-case emails, restore `.lower()` on `workEmail`
and `supervisorEmail` below or fix the downstream view to compare
case-insensitively.

Both tables use ReplacingMergeTree; we TRUNCATE before each insert so
re-runs stay clean. `class_people` is a versionless RMT (its dbt model
`silver/_shared/class_people.sql` unions staging with
`dedup_version_col=none`), so the insert below deliberately omits a
`_version` column.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING

from generators.base import bulk_insert, deterministic_uuid, truncate
from profiles import Person

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


_TEAM_DEPARTMENT = {
    "development": "Development",
    "sales":       "Sales",
    "hr":          "HR",
    "support":     "Support",
}

_TEAM_DIVISION = {
    "development": "Engineering",
    "sales":       "Go-to-Market",
    "hr":          "People Ops",
    "support":     "Customer Success",
}

_JOB_TITLES = {
    ("development", "lead"):  "Engineering Manager",
    ("development", "ic"):    "Software Engineer",
    ("sales", "lead"):        "Sales Manager",
    ("sales", "ic"):          "Account Executive",
    ("hr", "lead"):           "HR Lead",
    ("hr", "ic"):             "People Partner",
    ("support", "lead"):      "Support Lead",
    ("support", "ic"):        "Support Engineer",
}


def _display_name(p: Person) -> str:
    """Person's real name, or a synthesized one from the email's local part.

    The roster now carries first/last names (profiles.py), so prefer those —
    this feeds bronze_bamboohr.employees.displayName, which the analytics
    `insight.team_member` view surfaces in the UI's Team Members table.
    """
    if p.display_name:
        return p.display_name
    local = p.email.split("@", 1)[0]
    return local.replace("_", " ").replace(".", " ").title()


def _job_title(p: Person) -> str:
    if p.role == "ceo":
        return "Chief Executive Officer"
    if p.team is None:
        return ""
    return _JOB_TITLES.get((p.team, p.role), "Member")


def _supervisor_email(roster: Sequence[Person], p: Person) -> str | None:
    if not p.parent_uuid:
        return None
    for q in roster:
        if q.uuid == p.parent_uuid:
            return q.email
    return None


def seed_class_people(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> int:
    truncate(client, "silver", "class_people")
    # workspace_id carries the tenant: metric_entity_cohorts_current filters
    # `workspace_id IS NOT NULL AND workspace_id != ''` and selects it AS
    # tenant_id, so an empty value drops every person from the cohort view.
    # It MUST equal the tenant_uuid the git/ai generators use, or the
    # observation↔cohort join yields nothing.
    # `class_people` is a versionless ReplacingMergeTree (see module
    # docstring), so no `_version` column here — emitting one fails against
    # the dbt-materialised table, which has no such column.
    cols = ["unique_key", "email", "department_name", "workspace_id"]
    rows: list[tuple[object, ...]] = []
    for p in roster:
        dept = _TEAM_DEPARTMENT.get(p.team or "", "Executive")
        rows.append((
            deterministic_uuid("class_people", p.email),
            p.email.lower(),
            dept,
            tenant_uuid,
        ))
    return bulk_insert(client, "silver", "class_people", cols, rows)


def seed_bamboohr_employees(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
) -> int:
    truncate(client, "bronze_bamboohr", "employees")
    cols = [
        "id",
        "status",
        "firstName",
        "lastName",
        "displayName",
        "workEmail",
        "department",
        "division",
        "jobTitle",
        "supervisorEmail",
        "supervisor",
    ]
    rows: list[tuple[object, ...]] = []
    for p in roster:
        full = _display_name(p)
        first, _, last = full.partition(" ")
        sup_email = _supervisor_email(roster, p)
        sup_name = ""
        if sup_email is not None:
            sup = next(q for q in roster if q.email == sup_email)
            sup_name = _display_name(sup)
        rows.append((
            deterministic_uuid("bamboohr.employee", p.email),
            "Active",
            first or full,
            last or "",
            full,
            p.email,
            _TEAM_DEPARTMENT.get(p.team or "", "Executive"),
            _TEAM_DIVISION.get(p.team or "", "Executive"),
            _job_title(p),
            (sup_email or ""),
            sup_name,
        ))
    return bulk_insert(client, "bronze_bamboohr", "employees", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> dict[str, int]:
    return {
        "silver.class_people":           seed_class_people(client, roster, tenant_uuid),
        "bronze_bamboohr.employees":     seed_bamboohr_employees(client, roster),
    }
