"""Tests for the task-tracking generator (`generators/task.py`).

The trap these tests pin: `task_issue_state` gold classifies an issue by
joining its issuetype field-history value_id against the
`class_task_issuetypes` dimension. A missing dimension row — or a
history event without a value_id — leaves the bug / non-bug measures
empty while every row still "looks" seeded.
"""

from __future__ import annotations

import datetime as _dt
from typing import Any

import pytest

from insight_seed import profiles
from insight_seed.generators import base, task

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 8, 11)
_DAYS = 60
# INVARIANT: mirrors the generator's rng.randint(3, 28) close delay — issues
# older than this are closable regardless of the drawn delay, so only they
# see the pure 55% close share.
_MAX_DAYS_TO_CLOSE = 28

_NO_CLIENT: Any = None

Rows = dict[str, list[dict[str, Any]]]
Issues = dict[str, list[dict[str, Any]]]

_TABLES = (
    "silver.class_task_worklogs",
    "silver.class_task_users",
    "silver.class_task_field_history",
    "silver.class_task_statuses",
    "silver.class_task_issuetypes",
)

_INITIAL_FIELDS = {
    "status",
    "assignee",
    "issuetype",
    "priority",
    "duedate",
    "timeoriginalestimate",
    "timespent",
}

_BUG_TYPE_ID = "10004"


def _capture_generate(mp: pytest.MonkeyPatch, roster: list[profiles.Person]) -> Rows:
    captured: Rows = {}

    def capture(
        client: Any, schema: str, table: str, cols: list[str], data: list[tuple[Any, ...]]
    ) -> int:
        captured[f"{schema}.{table}"] = [dict(zip(cols, row, strict=True)) for row in data]
        return len(data)

    mp.setattr(task, "truncate", lambda client, schema, table: None)
    mp.setattr(task, "bulk_insert", capture)
    task.generate(_NO_CLIENT, roster, _TENANT, _DAYS)
    return captured


@pytest.fixture(scope="module")
def roster() -> list[profiles.Person]:
    return profiles.build_roster("dev@company.nonpresent")


@pytest.fixture(scope="module")
def runs(roster: list[profiles.Person]) -> tuple[Rows, Rows]:
    with pytest.MonkeyPatch.context() as mp:
        mp.setattr(base, "_anchor_cache", _ANCHOR)
        return _capture_generate(mp, roster), _capture_generate(mp, roster)


@pytest.fixture(scope="module")
def rows(runs: tuple[Rows, Rows]) -> Rows:
    return runs[0]


@pytest.fixture(scope="module")
def history(rows: Rows) -> list[dict[str, Any]]:
    return rows["silver.class_task_field_history"]


@pytest.fixture(scope="module")
def issues(history: list[dict[str, Any]]) -> Issues:
    grouped: Issues = {}
    for row in history:
        grouped.setdefault(row["issue_id"], []).append(row)
    return grouped


def _issuetype_initial(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [
        e for e in events if e["event_kind"] == "synthetic_initial" and e["field_id"] == "issuetype"
    ]


def _close_events(events: list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [e for e in events if e["event_kind"] == "changelog"]


# ─── Determinism ─────────────────────────────────────────────────────────


@pytest.mark.parametrize("table", _TABLES)
def test_a_rerun_over_the_same_roster_and_window_reproduces_every_row(
    runs: tuple[Rows, Rows], table: str
) -> None:
    first, second = runs
    assert first[table], f"{table} was seeded empty"
    assert first[table] == second[table], f"rows moved between identical runs: {table}"


# ─── Field-history row shape ─────────────────────────────────────────────


def test_every_issue_opens_with_an_issuetype_event_that_carries_a_value_id(
    issues: Issues,
) -> None:
    """The docstring'd trap: an issuetype event without a value_id cannot join
    the dimension, and gold reads the issue as an unclassified type."""
    for issue_id, events in issues.items():
        typed = _issuetype_initial(events)
        assert len(typed) == 1, f"issue {issue_id} has {len(typed)} issuetype initials"
        event = typed[0]
        assert event["delta_value_id"], f"issue {issue_id}: issuetype event has no value_id"
        assert event["value_ids"] == [event["delta_value_id"]], (
            f"issue {issue_id}: value_ids diverges from delta_value_id"
        )
        assert event["value_id_type"] == "string_literal", issue_id


def test_every_issue_gets_one_synthetic_initial_row_per_tracked_field(
    issues: Issues,
) -> None:
    for issue_id, events in issues.items():
        initial = [e for e in events if e["event_kind"] == "synthetic_initial"]
        fields = [e["field_id"] for e in initial]
        assert set(fields) == _INITIAL_FIELDS, f"issue {issue_id} initial fields: {sorted(fields)}"
        assert len(fields) == len(_INITIAL_FIELDS), f"issue {issue_id} duplicates a field"


def test_a_close_event_flips_status_to_a_done_status_with_its_dimension_id(
    issues: Issues,
) -> None:
    for issue_id, events in issues.items():
        for event in _close_events(events):
            assert event["field_id"] == "status", f"issue {issue_id} changelog is not a status"
            display = event["delta_value_display"]
            assert display in task._CLOSE_STATUSES, f"issue {issue_id} closes to {display!r}"
            assert event["delta_value_id"] == task._STATUS_DIM[display][0], (
                f"issue {issue_id}: close value_id does not match the {display!r} dimension row"
            )
            assert event["event_at"].date() < _ANCHOR, f"issue {issue_id} closes after the anchor"


def test_roughly_the_configured_share_of_old_enough_issues_is_closed(
    issues: Issues,
) -> None:
    """Issues older than the max close delay are closable whatever delay the
    rng drew, so only there is the 55% share observable undiluted."""
    cutoff = _ANCHOR - _dt.timedelta(days=_MAX_DAYS_TO_CLOSE + 1)
    old = {
        issue_id: events
        for issue_id, events in issues.items()
        if events[0]["event_at"].date() <= cutoff
    }
    assert len(old) >= 100, f"only {len(old)} old issues; widen _DAYS"

    closed = sum(1 for events in old.values() if _close_events(events))
    share = closed / len(old)
    assert 0.45 <= share <= 0.65, f"close share {share:.3f} strays from the configured 0.55"


# ─── Issue-type distribution ─────────────────────────────────────────────


def _issue_types(issues: Issues) -> dict[str, str]:
    return {
        issue_id: _issuetype_initial(events)[0]["delta_value_id"]
        for issue_id, events in issues.items()
    }


@pytest.mark.parametrize("type_name", task._ISSUE_TYPES)
def test_every_issue_type_appears_across_the_roster(issues: Issues, type_name: str) -> None:
    type_id = task._ISSUE_TYPE_DIM[type_name][0]
    seeded = [t for t in _issue_types(issues).values() if t == type_id]
    assert seeded, f"no issue seeded with type {type_name!r} ({type_id})"


def test_bugs_are_roughly_a_quarter_of_all_issues(issues: Issues) -> None:
    types = _issue_types(issues)
    assert len(types) >= 300, f"only {len(types)} issues; widen _DAYS or roster"

    share = sum(1 for t in types.values() if t == _BUG_TYPE_ID) / len(types)
    assert 0.19 <= share <= 0.31, f"bug share {share:.3f} strays from the uniform 0.25"


def test_closed_bugs_exist_so_bugs_fixed_is_non_zero(issues: Issues) -> None:
    types = _issue_types(issues)
    closed_bugs = [
        issue_id
        for issue_id, events in issues.items()
        if types[issue_id] == _BUG_TYPE_ID and _close_events(events)
    ]
    assert closed_bugs, "no closed bug in the whole window — bugs_fixed would read zero"


# ─── Issue-type dimension ────────────────────────────────────────────────


def test_the_issuetype_dimension_emits_exactly_the_declared_rows_per_source(
    rows: Rows,
) -> None:
    by_source: dict[str, dict[str, tuple[str, str]]] = {}
    for row in rows["silver.class_task_issuetypes"]:
        per_source = by_source.setdefault(row["insight_source_id"], {})
        assert row["issue_type_id"] not in per_source, (
            f"source {row['insight_source_id']} duplicates type {row['issue_type_id']}"
        )
        per_source[row["issue_type_id"]] = (row["issue_type_name"], row["issue_kind"])

    declared = {type_id: (name, kind) for name, (type_id, kind) in task._ISSUE_TYPE_DIM.items()}
    for source_id, per_source in by_source.items():
        assert per_source == declared, f"source {source_id} diverges from _ISSUE_TYPE_DIM"


def test_the_dimension_carries_a_bug_kind_row_keyed_by_the_history_value_id(
    rows: Rows,
) -> None:
    bug_rows = [r for r in rows["silver.class_task_issuetypes"] if r["issue_kind"] == "bug"]
    assert bug_rows, "no bug-kind dimension row — gold cannot classify any bug"
    for row in bug_rows:
        assert row["issue_type_id"] == _BUG_TYPE_ID, (
            f"bug row keyed {row['issue_type_id']!r}, history events use {_BUG_TYPE_ID!r}"
        )
        assert row["issue_type_name"] == "Bug"


# ─── Referential integrity of the seed itself ────────────────────────────


def test_every_issuetype_value_id_in_history_exists_in_the_dimension(
    rows: Rows, history: list[dict[str, Any]]
) -> None:
    """The join gold depends on: (insight_source_id, issue_type_id)."""
    dimension = {
        (r["insight_source_id"], r["issue_type_id"]) for r in rows["silver.class_task_issuetypes"]
    }
    for event in history:
        if event["field_id"] != "issuetype":
            continue
        key = (event["insight_source_id"], event["delta_value_id"])
        assert key in dimension, f"issue {event['issue_id']} references unseeded type {key}"


def test_every_status_value_id_in_history_exists_in_the_status_dimension(
    rows: Rows, history: list[dict[str, Any]]
) -> None:
    """Same trap one dimension over: an unmatched status_id yields a NULL
    status_category and the closed-task measures go empty."""
    dimension = {
        (r["insight_source_id"], r["status_id"]) for r in rows["silver.class_task_statuses"]
    }
    for event in history:
        if event["field_id"] != "status":
            continue
        key = (event["insight_source_id"], event["delta_value_id"])
        assert key in dimension, f"issue {event['issue_id']} references unseeded status {key}"
