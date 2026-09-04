"""The issue title reaches gold through the `title` role.

This is the one part of the change whose consumer is gold rather than the
journal, and it has a transitional shape that only a test can hold in place:

  * GitHub emits the title as a FIELD, so the role answers for it today;
  * Jira's producer is still the Rust binary, which fills the denormalized
    `title` COLUMN on every row but emits a `summary` field row only for an
    issue whose summary actually changed — the snapshot model it reads does not
    list `summary`. So a never-renamed Jira issue is named by the column alone;
  * gold therefore reads the role first and falls back to the column, and both
    the column and the fallback go with the binary at cutover.

Get the precedence backwards and nothing fails loudly: titles quietly become
empty for most Jira issues, or a rename stops showing. Hence the test.

Seeds `silver.class_task_field_history` directly rather than going through
bronze: the subject is what GOLD does with the two channels, and building the
whole Jira chain to arrange one row would test the chain instead.
"""

from __future__ import annotations

import pytest

SOURCE = "jira-title-role-test"
TENANT = "11111111-1111-1111-1111-111111111111"
EMAIL = "title-role@example.com"

HISTORY_TABLE = "silver.class_task_field_history"


def row(
    issue: str,
    field_id: str,
    *,
    column_title: str | None,
    field_value: str | None,
    seq: int = 1,
    kind: str = "synthetic_initial",
    at: str = "2026-01-05 09:00:00",
    data_source: str = "jira",
) -> dict:
    """One journal row: `column_title` is the denormalized column, `field_value`
    the value the field itself carries."""
    values = [] if field_value is None else [field_value]
    return {
        "unique_key": f"{issue}-{field_id}-{seq}",
        "insight_source_id": SOURCE,
        "data_source": data_source,
        "issue_id": issue,
        "id_readable": issue,
        "title": column_title,
        "event_id": f"initial:{issue}",
        "event_at": at,
        "event_kind": kind,
        "_seq": seq,
        "author_id": "actor",
        "field_id": field_id,
        "field_name": field_id,
        "field_cardinality": "single",
        "delta_action": "set",
        "value_ids": values,
        "value_displays": values,
        "value_id_type": "none",
        "collected_at": at,
        "_version": 1,
    }


def assignee_row(issue: str, *, column_title: str | None, data_source: str = "jira") -> dict:
    """Gold reaches a person through the assignee role — `task_issue_state`
    inner-joins `class_task_users` on it. Without this row the issue never
    reaches the serving table, and a test would fail for a reason that has
    nothing to do with titles."""
    return row(issue, "assignee", column_title=column_title, field_value="actor", seq=9, data_source=data_source)


@pytest.fixture
def gold(warehouse):
    """A clean slate for this source, and an actor gold can attribute to."""
    warehouse.execute(
        "DELETE FROM silver.class_task_field_history WHERE insight_source_id = {src:String}", {"src": SOURCE}
    )
    warehouse.execute("DELETE FROM silver.class_task_users WHERE insight_source_id = {src:String}", {"src": SOURCE})
    warehouse.insert(
        "silver.class_task_users",
        [
            {
                "unique_key": f"{SOURCE}-actor",
                "tenant_id": TENANT,
                "insight_source_id": SOURCE,
                "data_source": "jira",
                "user_id": "actor",
                "email": EMAIL,
                "display_name": "Title Role",
                "collected_at": "2026-01-05 09:00:00",
                "_version": 1,
            }
        ],
    )
    return warehouse


def seed_and_build(warehouse, rows: list[dict]) -> dict[str, str]:
    warehouse.insert(HISTORY_TABLE, rows)
    # The role view is a parent gold reads by name, and `+task_issue_state` would
    # drag in the silver unions, whose other arms this warehouse does not have.
    warehouse.dbt("run", "--select", "task_field_roles_current", "task_issue_state", "--full-refresh")
    return {
        r["id_readable"]: r["title"]
        for r in warehouse.rows(
            "SELECT id_readable, title FROM insight.task_issue_state WHERE insight_source_id = {src:String}",
            {"src": SOURCE},
        )
    }


def test_the_role_wins_over_the_column(gold):
    """When both channels answer, the role is the one that counts.

    The column is a snapshot of the issue's summary as the binary last saw it;
    the role is the journal's own latest value. Reading the column first would
    make a rename invisible for as long as the binary keeps writing.
    """
    titles = seed_and_build(
        gold,
        [
            row("ROLE-1", "created", column_title="stale from the column", field_value=None, seq=0),
            row("ROLE-1", "summary", column_title="stale from the column", field_value="renamed, from the role", seq=1),
            assignee_row("ROLE-1", column_title="stale from the column"),
        ],
    )
    assert titles["ROLE-1"] == "renamed, from the role"


def test_the_column_carries_an_issue_the_role_cannot_name(gold):
    """A Jira issue never renamed: the binary emits no `summary` value, so the
    role has nothing to offer and the column is the only title there is.

    This is the case that made dropping the column a regression — it is the
    ordinary case, not the exception.
    """
    titles = seed_and_build(
        gold,
        [
            row("ROLE-2", "created", column_title="named by the column only", field_value=None, seq=0),
            row("ROLE-2", "summary", column_title="named by the column only", field_value=None, seq=1),
            assignee_row("ROLE-2", column_title="named by the column only"),
        ],
    )
    assert titles["ROLE-2"] == "named by the column only"


def test_a_source_with_no_column_is_named_by_the_role(gold):
    """The shape after cutover: no column value at all, and the role answers.

    Modelled on Jira rather than GitHub because gold reaches a person through
    the assignee role, and GitHub's assignee binding is per-installation
    configuration rather than a vendor default — seeding one here would test
    the configuration, not the title.
    """
    titles = seed_and_build(
        gold,
        [
            row("ROLE-3", "created", column_title=None, field_value=None, seq=0),
            row("ROLE-3", "summary", column_title=None, field_value="named by the role", seq=1),
            assignee_row("ROLE-3", column_title=None),
        ],
    )
    assert titles["ROLE-3"] == "named by the role"


def test_an_issue_with_neither_reads_as_null(gold):
    """Neither channel answers: NULL, not an empty string. `argMaxIf` returns
    the type default when nothing matches, and this is a serving column the
    backend reads — its nullability is part of the contract."""
    titles = seed_and_build(
        gold,
        [
            row("ROLE-4", "created", column_title=None, field_value=None, seq=0),
            row("ROLE-4", "status", column_title=None, field_value="6", seq=1),
            assignee_row("ROLE-4", column_title=None),
        ],
    )
    assert titles["ROLE-4"] is None
