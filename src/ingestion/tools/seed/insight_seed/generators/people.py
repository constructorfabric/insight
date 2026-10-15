"""
People / org-linkage seed.

Populates:

* `silver.class_people` — one row per person with `department_name`
  set to the team. Cohort and coverage gold models join on this.
* `bronze_bamboohr.employees` — emails + departments + supervisorEmail
  so the HR directory can resolve `org_unit_id` and the supervisor
  chain from a BambooHR-shaped table. The columns used are workEmail,
  displayName, department, jobTitle, supervisorEmail.

`silver.class_people` lowercases its `email` column so case-insensitive
joins downstream match cleanly. `bronze_bamboohr.employees` keeps the
original casing that a real BambooHR feed would deliver — fine here
because the seed roster (`profiles.py`) already uses lowercase
addresses end-to-end, so no identities split in practice. If a future
roster introduces mixed-case emails, restore `.lower()` on `workEmail`
and `supervisorEmail` below or fix the downstream model to compare
case-insensitively.

Both tables use ReplacingMergeTree; we TRUNCATE before each insert so
re-runs stay clean. `class_people` is a versionless RMT (its dbt model
`silver/_shared/class_people.sql` unions staging with
`dedup_version_col=none`), so the insert below deliberately omits a
`_version` column.
"""

from __future__ import annotations

import json
from collections.abc import Sequence
from typing import TYPE_CHECKING

from ..profiles import Person
from .base import deterministic_uuid
from .insert import bulk_insert, truncate

# Identity keys bindings on (source_type, source_id, account_id): one stable value must span claims and bindings.
_BAMBOOHR_SOURCE_ID = "seed-bamboohr"

if TYPE_CHECKING:
    import clickhouse_connect.driver.client


_TEAM_DEPARTMENT = {
    "development": "Development",
    "sales": "Sales",
    "hr": "HR",
    "support": "Support",
}

_TEAM_DIVISION = {
    "development": "Engineering",
    "sales": "Go-to-Market",
    "hr": "People Ops",
    "support": "Customer Success",
}

_JOB_TITLES = {
    ("development", "lead"): "Engineering Manager",
    ("development", "ic"): "Software Engineer",
    ("sales", "lead"): "Sales Manager",
    ("sales", "ic"): "Account Executive",
    ("hr", "lead"): "HR Lead",
    ("hr", "ic"): "People Partner",
    ("support", "lead"): "Support Lead",
    ("support", "ic"): "Support Engineer",
}


def _display_name(p: Person) -> str:
    """Person's real name, or a synthesized one from the email's local part.

    The roster now carries first/last names (profiles.py), so prefer those —
    this feeds bronze_bamboohr.employees.displayName, which reaches the UI
    through the HR directory.
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


def _measured_persons(roster: Sequence[Person]) -> list[Person]:
    """The people the product measures — the organisation, not its operators.

    The admin operator is in the roster so it can log in and administer the
    API, but it is not an employee: including it would add a headcount and a
    BambooHR employee record for an account nobody works at. Every other
    generator excludes it implicitly by filtering on `team`; these two iterate
    the whole roster (the CEO is teamless and must stay in), so the exclusion
    has to be explicit.
    """
    return [p for p in roster if p.role != "admin"]


def seed_bamboohr_employees(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
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
        "tenant_id",
        "source_id",
        "raw_data",
    ]
    rows: list[tuple[object, ...]] = []
    for p in _measured_persons(roster):
        full = _display_name(p)
        first, _, last = full.partition(" ")
        sup_email = _supervisor_email(roster, p)
        sup_name = ""
        if sup_email is not None:
            sup = next(q for q in roster if q.email == sup_email)
            sup_name = _display_name(sup)

        # The identity chain diffs raw_data, not the typed columns: NULL emits no identity observation.
        record = {
            "id": deterministic_uuid("bamboohr.employee", p.email),
            "status": "Active",
            "firstName": first or full,
            "lastName": last or "",
            "displayName": full,
            "workEmail": p.email,
            "department": _TEAM_DEPARTMENT.get(p.team or "", "Executive"),
            "division": _TEAM_DIVISION.get(p.team or "", "Executive"),
            "jobTitle": _job_title(p),
            "supervisorEmail": sup_email or "",
            "supervisor": sup_name,
        }
        rows.append(
            (
                record["id"],
                record["status"],
                record["firstName"],
                record["lastName"],
                record["displayName"],
                record["workEmail"],
                record["department"],
                record["division"],
                record["jobTitle"],
                record["supervisorEmail"],
                record["supervisor"],
                tenant_uuid,
                _BAMBOOHR_SOURCE_ID,
                json.dumps(record),
            )
        )
    return bulk_insert(client, "bronze_bamboohr", "employees", cols, rows)


def generate(
    client: clickhouse_connect.driver.client.Client,
    roster: Sequence[Person],
    tenant_uuid: str,
) -> dict[str, int]:
    # Cleared, not written: `bamboohr__to_class_people` and
    # `bamboohr__working_hours` build these from the employees below, and
    # `class_focus_metrics` is derived from working hours and meeting activity.
    # A re-seed at another anchor would otherwise leave the old window behind
    # the incremental watermark.
    truncate(client, "silver", "class_people")
    truncate(client, "silver", "class_hr_working_hours")
    truncate(client, "silver", "class_focus_metrics")
    truncate(client, "staging", "bamboohr__to_class_people")
    truncate(client, "staging", "bamboohr__working_hours")

    return {
        "bronze_bamboohr.employees": seed_bamboohr_employees(client, roster, tenant_uuid),
    }
