"""An issue closes when it last ENTERS a done-category status, not at its last done event.

A move between two done statuses (Resolved → Closed, Closed → Verified) is
bookkeeping after the close; counting it would shift the closed date and every
duration that ends there. A reopen followed by a fresh close is a new entry and
moves the close forward. An issue created in a done status entered it at birth.

Seeds `silver.class_task_field_history`, `class_task_statuses` and
`class_task_users` directly: the subject is what gold does with them, as in
`test_title_role`.
"""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Any, NamedTuple

import pytest
from conftest import Warehouse

SOURCE = "jira-final-close-test"
TENANT = "33333333-3333-3333-3333-333333333333"
ACTOR = "close-actor"

OPEN = "1"
IN_PROGRESS = "3"
REOPENED = "4"
RESOLVED = "5"
CLOSED = "6"
VERIFIED = "10"

STATUSES = {
    OPEN: "new",
    IN_PROGRESS: "in_progress",
    REOPENED: "new",
    RESOLVED: "done",
    CLOSED: "done",
    VERIFIED: "done",
}

ORDER_BAND_INITIAL = 0
ORDER_BAND_CHANGELOG = 1


class Lifecycle(NamedTuple):
    issue: str
    created_in: str
    moves: list[tuple[str, str]]
    closes_at: datetime | None


LIFECYCLES = [
    Lifecycle(
        "CLOSE-1",
        OPEN,
        [("2026-03-02 09:00:00", IN_PROGRESS), ("2026-03-03 09:00:00", RESOLVED), ("2026-03-09 09:00:00", CLOSED)],
        datetime(2026, 3, 3, 9),
    ),
    Lifecycle(
        "CLOSE-2",
        OPEN,
        [("2026-03-03 09:00:00", RESOLVED), ("2026-03-09 09:00:00", CLOSED), ("2026-03-12 09:00:00", VERIFIED)],
        datetime(2026, 3, 3, 9),
    ),
    Lifecycle(
        "REOPEN-1",
        OPEN,
        [
            ("2026-03-03 09:00:00", RESOLVED),
            ("2026-03-04 09:00:00", REOPENED),
            ("2026-03-06 09:00:00", RESOLVED),
            ("2026-03-09 09:00:00", CLOSED),
        ],
        datetime(2026, 3, 6, 9),
    ),
    Lifecycle("BORN-DONE-1", CLOSED, [], datetime(2026, 3, 1, 9)),
    Lifecycle("BORN-DONE-2", CLOSED, [("2026-03-09 09:00:00", VERIFIED)], datetime(2026, 3, 1, 9)),
    Lifecycle("OPEN-1", OPEN, [("2026-03-02 09:00:00", IN_PROGRESS)], None),
]

CREATED_AT = "2026-03-01 09:00:00"


def event_order(at: str, band: int, rank: int) -> int:
    millis = int(datetime.fromisoformat(at).replace(tzinfo=UTC).timestamp() * 1000)
    return millis * 1_000_000 + band * 100_000 + rank


def journal_row(issue: str, field_id: str, value: str | None, *, at: str, rank: int, changelog: bool) -> dict[str, Any]:
    values = [] if value is None else [value]
    band = ORDER_BAND_CHANGELOG if changelog else ORDER_BAND_INITIAL
    event_id = f"{issue}-{at}" if changelog else f"initial:{issue}"
    return {
        "unique_key": f"{issue}-{field_id}-{event_id}",
        "insight_source_id": SOURCE,
        "data_source": "jira",
        "issue_id": issue,
        "id_readable": issue,
        "event_id": event_id,
        "event_at": at,
        "event_kind": "changelog" if changelog else "synthetic_initial",
        "event_order": event_order(at, band, rank),
        "_seq": rank,
        "author_id": ACTOR,
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


def journal(lifecycle: Lifecycle) -> list[dict[str, Any]]:
    initial = [
        journal_row(lifecycle.issue, "created", None, at=CREATED_AT, rank=0, changelog=False),
        journal_row(lifecycle.issue, "status", lifecycle.created_in, at=CREATED_AT, rank=1, changelog=False),
        journal_row(lifecycle.issue, "assignee", ACTOR, at=CREATED_AT, rank=2, changelog=False),
    ]
    moves = [journal_row(lifecycle.issue, "status", s, at=at, rank=0, changelog=True) for at, s in lifecycle.moves]
    return initial + moves


def status_row(status_id: str, category: str) -> dict[str, Any]:
    return {
        "unique_key": f"{SOURCE}-status-{status_id}",
        "insight_source_id": SOURCE,
        "data_source": "jira",
        "status_id": status_id,
        "status_name": f"status {status_id}",
        "status_category": category,
        "collected_at": CREATED_AT,
        "_version": 1,
    }


@pytest.fixture(scope="module")
def built(warehouse: Warehouse) -> Warehouse:
    for table in ("class_task_field_history", "class_task_users", "class_task_statuses"):
        warehouse.execute(f"DELETE FROM silver.{table} WHERE insight_source_id = {{src:String}}", {"src": SOURCE})

    warehouse.insert(
        "silver.class_task_users",
        [
            {
                "unique_key": f"{SOURCE}-{ACTOR}",
                "tenant_id": TENANT,
                "insight_source_id": SOURCE,
                "data_source": "jira",
                "user_id": ACTOR,
                "email": "close-actor@example.com",
                "display_name": "Close Actor",
                "collected_at": CREATED_AT,
                "_version": 1,
            }
        ],
    )
    warehouse.insert("silver.class_task_statuses", [status_row(s, c) for s, c in STATUSES.items()])
    warehouse.insert("silver.class_task_field_history", [r for lc in LIFECYCLES for r in journal(lc)])

    # The role view is a parent gold reads by name, and `+task_issue_state` would
    # drag in the silver unions, whose other arms this warehouse does not have.
    warehouse.dbt(
        "run", "--select", "task_field_roles_current", "task_issue_state", "task_status_spans", "--full-refresh"
    )
    return warehouse


@pytest.mark.parametrize("lifecycle", LIFECYCLES, ids=[lc.issue for lc in LIFECYCLES])
def test_an_issue_closes_at_its_last_entry_into_done(built: Warehouse, lifecycle: Lifecycle) -> None:
    rows = built.rows(
        "SELECT final_close_at FROM insight.task_issue_state"
        " WHERE insight_source_id = {src:String} AND issue_id = {issue:String}",
        {"src": SOURCE, "issue": lifecycle.issue},
    )

    assert [r["final_close_at"] for r in rows] == [lifecycle.closes_at], f"wrong close for {lifecycle.issue}"


def test_a_move_between_done_statuses_keeps_its_span(built: Warehouse) -> None:
    """The tail span of a done issue must not be cut at the close: after a
    Resolved → Closed move it starts after the close, and ending it there would
    drop the span from the timeline."""
    spans = built.rows(
        "SELECT interval_start, status_category, duration_seconds FROM insight.task_status_spans"
        " WHERE insight_source_id = {src:String} AND issue_id = 'CLOSE-1' ORDER BY interval_start",
        {"src": SOURCE},
    )

    assert [(s["interval_start"], s["status_category"], s["duration_seconds"]) for s in spans] == [
        (datetime(2026, 3, 1, 9), "new", 86400.0),
        (datetime(2026, 3, 2, 9), "in_progress", 86400.0),
        (datetime(2026, 3, 3, 9), "done", 6 * 86400.0),
        (datetime(2026, 3, 9, 9), "done", 0.0),
    ]
