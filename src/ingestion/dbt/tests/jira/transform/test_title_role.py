"""The issue title reaches gold through the `title` role, and through nothing else.

The title is an ordinary field: Jira's producer emits `summary` for every issue
(a `synthetic_initial` row from the issue JSON plus its changelog events), GitHub
emits `title`, and both bind to the `title` role. Gold reads the role's newest
value; the denormalized `title` column the class once carried is gone.

Get the ordering wrong and nothing fails loudly: a rename stops showing, or an
issue reads as its original name. Hence the test.

Seeds `silver.class_task_field_history` directly rather than going through
bronze: the subject is what GOLD does with the journal, and building the whole
Jira chain to arrange one row would test the chain instead.
"""

from __future__ import annotations

from typing import Any

import pytest
from conftest import Warehouse

SOURCE = "jira-title-role-test"
TENANT = "11111111-1111-1111-1111-111111111111"
EMAIL = "title-role@example.com"

HISTORY_TABLE = "silver.class_task_field_history"


def row(
    issue: str,
    field_id: str,
    *,
    field_value: str | None,
    seq: int = 1,
    kind: str = "synthetic_initial",
    event_id: str | None = None,
    at: str = "2026-01-05 09:00:00",
) -> dict[str, Any]:
    """One journal row carrying `field_value` as the field's value."""
    values = [] if field_value is None else [field_value]
    return {
        "unique_key": f"{issue}-{field_id}-{seq}-{event_id or 'initial'}",
        "insight_source_id": SOURCE,
        "data_source": "jira",
        "issue_id": issue,
        "id_readable": issue,
        "event_id": event_id or f"initial:{issue}",
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


def assignee_row(issue: str) -> dict[str, Any]:
    """Gold reaches a person through the assignee role — `task_issue_state`
    inner-joins `class_task_users` on it. Without this row the issue never
    reaches the serving table, and a test would fail for a reason that has
    nothing to do with titles."""
    return row(issue, "assignee", field_value="actor", seq=9)


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


def seed_and_build(warehouse, rows: list[dict[str, Any]]) -> dict[str, str]:
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


def test_the_newest_summary_names_the_issue(gold: Warehouse) -> None:
    """A renamed issue is named by its latest summary, not its first.

    The initial row states the name at creation; the changelog row states the
    rename. Reading the wrong one would freeze every issue at its original name.
    """
    titles = seed_and_build(
        gold,
        [
            row("ROLE-1", "created", field_value=None, seq=0),
            row("ROLE-1", "summary", field_value="the original name", seq=1),
            row(
                "ROLE-1",
                "summary",
                field_value="renamed later",
                seq=0,
                kind="changelog",
                event_id="101",
                at="2026-01-06 09:00:00",
            ),
            assignee_row("ROLE-1"),
        ],
    )
    assert titles["ROLE-1"] == "renamed later"


def test_a_never_renamed_issue_is_named_by_its_initial_summary(gold: Warehouse) -> None:
    """The ordinary case: one `summary` row, from the issue JSON at creation.

    This is the row the derived model emits for every issue — the reason the
    denormalized column could go without leaving never-renamed issues unnamed.
    """
    titles = seed_and_build(
        gold,
        [
            row("ROLE-2", "created", field_value=None, seq=0),
            row("ROLE-2", "summary", field_value="named once", seq=1),
            assignee_row("ROLE-2"),
        ],
    )
    assert titles["ROLE-2"] == "named once"


def test_an_issue_without_a_summary_reads_as_null(gold: Warehouse) -> None:
    """No `summary` row: NULL, not an empty string. `argMaxIf` returns the type
    default when nothing matches, and this is a serving column the backend
    reads — its nullability is part of the contract."""
    titles = seed_and_build(
        gold,
        [
            row("ROLE-4", "created", field_value=None, seq=0),
            row("ROLE-4", "status", field_value="6", seq=1),
            assignee_row("ROLE-4"),
        ],
    )
    assert titles["ROLE-4"] is None
