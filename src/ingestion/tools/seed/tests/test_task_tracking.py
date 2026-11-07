"""Tests for the task-tracking generator (`generators/task.py`).

The trap these tests pin: `task_issue_state` gold classifies an issue by
joining its issuetype field-history value_id against the operator's
`config.field_value_map` rows (the `class_task_issuetypes` dimension carries
the names). A missing mapping row — or a history event without a value_id —
leaves the bug / non-bug measures empty while every row still "looks" seeded.
"""

from __future__ import annotations

import datetime as _dt
import json
import re
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
    "bronze_jira.jira_issue",
    "bronze_jira.jira_issue_history",
    "bronze_jira.jira_worklogs",
    "bronze_jira.jira_user",
    "bronze_jira.jira_statuses",
    "bronze_jira.jira_issuetypes",
    "bronze_jira.jira_fields",
    "config.field_value_map",
    "config.field_value_defaults",
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
def issues(rows: Rows) -> dict[str, dict[str, Any]]:
    """Each issue by its Jira id, with its field snapshot already parsed."""
    parsed: dict[str, dict[str, Any]] = {}
    for row in rows["bronze_jira.jira_issue"]:
        fields = json.loads(row["custom_fields_json"])
        parsed[row["jira_id"]] = {**row, "fields": fields}
    return parsed


@pytest.fixture(scope="module")
def closes(rows: Rows) -> dict[str, list[dict[str, Any]]]:
    """The changelog entries that close an issue, by Jira id."""
    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in rows["bronze_jira.jira_issue_history"]:
        for item in json.loads(row["items"]):
            if item["field"] == "status":
                grouped.setdefault(row["jira_id"], []).append(item)
    return grouped


# ─── Determinism ─────────────────────────────────────────────────────────


@pytest.mark.parametrize("table", _TABLES)
def test_a_rerun_over_the_same_roster_and_window_reproduces_every_row(
    runs: tuple[Rows, Rows], table: str
) -> None:
    first, second = runs
    assert first[table], f"{table} was seeded empty"
    assert first[table] == second[table], f"rows moved between identical runs: {table}"


# ─── Field-history row shape ─────────────────────────────────────────────


def test_every_issue_snapshot_carries_an_issuetype_id(
    issues: dict[str, dict[str, Any]],
) -> None:
    """The docstring'd trap, one layer earlier: the journal derives the
    issuetype event from this snapshot, and an id the dimension does not carry
    leaves gold reading the issue as an unclassified type."""
    for issue_id, issue in issues.items():
        issuetype = issue["fields"].get("issuetype")
        assert issuetype, f"issue {issue_id} carries no issuetype"
        assert issuetype["id"], f"issue {issue_id}: issuetype has no id"
        assert issuetype["name"] in task._ISSUE_TYPE_DIM, (
            f"issue {issue_id}: issuetype {issuetype['name']!r} is not in the dimension"
        )


def test_every_issue_snapshot_carries_each_tracked_field(
    issues: dict[str, dict[str, Any]],
) -> None:
    """`jira__issue_field_snapshot` unpivots exactly this object, so a field
    missing here produces no synthetic_initial row for it."""
    for issue_id, issue in issues.items():
        assert set(issue["fields"]) == _INITIAL_FIELDS, (
            f"issue {issue_id} snapshot fields: {sorted(issue['fields'])}"
        )


def test_a_closed_issue_snapshots_the_status_it_closed_into(
    issues: dict[str, dict[str, Any]], closes: dict[str, list[dict[str, Any]]]
) -> None:
    """INVARIANT: the journal derives an issue's opening status by walking the
    changelog back from the snapshot. A snapshot pinned to the opening status
    contradicts the close entry, and the issue never reaches a done category."""
    for issue_id, items in closes.items():
        assert len(items) == 1, f"issue {issue_id} has {len(items)} status changes"
        item = items[0]
        assert item["fromString"] == "To Do", f"issue {issue_id} does not open To Do"
        assert item["toString"] in task._CLOSE_STATUSES, (
            f"issue {issue_id} closes to {item['toString']!r}"
        )
        assert item["to"] == task._STATUS_DIM[item["toString"]][0], (
            f"issue {issue_id}: the close id does not match the dimension row"
        )
        assert issues[issue_id]["fields"]["status"]["name"] == item["toString"], (
            f"issue {issue_id}: the snapshot disagrees with the changelog"
        )


def test_roughly_the_configured_share_of_old_enough_issues_is_closed(
    issues: dict[str, dict[str, Any]], closes: dict[str, list[dict[str, Any]]]
) -> None:
    """Issues older than the max close delay are closable whatever delay the
    rng drew, so only there is the 55% share observable undiluted."""
    cutoff = _ANCHOR - _dt.timedelta(days=_MAX_DAYS_TO_CLOSE + 1)
    old = [
        issue_id
        for issue_id, issue in issues.items()
        if _dt.date.fromisoformat(issue["created"][:10]) <= cutoff
    ]
    assert len(old) >= 100, f"only {len(old)} old issues; widen _DAYS"

    share = sum(1 for issue_id in old if issue_id in closes) / len(old)
    assert 0.45 <= share <= 0.65, f"close share {share:.3f} strays from the configured 0.55"


# ─── Issue-type distribution ─────────────────────────────────────────────


def _issue_types(issues: dict[str, dict[str, Any]]) -> dict[str, str]:
    return {issue_id: issue["fields"]["issuetype"]["id"] for issue_id, issue in issues.items()}


@pytest.mark.parametrize("type_name", task._ISSUE_TYPES)
def test_every_issue_type_appears_across_the_roster(
    issues: dict[str, dict[str, Any]], type_name: str
) -> None:
    type_id = task._ISSUE_TYPE_DIM[type_name][0]
    seeded = [t for t in _issue_types(issues).values() if t == type_id]
    assert seeded, f"no issue seeded with type {type_name!r} ({type_id})"


def test_bugs_are_roughly_a_quarter_of_all_issues(issues: dict[str, dict[str, Any]]) -> None:
    types = _issue_types(issues)
    assert len(types) >= 300, f"only {len(types)} issues; widen _DAYS or roster"

    share = sum(1 for t in types.values() if t == _BUG_TYPE_ID) / len(types)
    assert 0.19 <= share <= 0.31, f"bug share {share:.3f} strays from the uniform 0.25"


def test_closed_bugs_exist_so_bugs_fixed_is_non_zero(
    issues: dict[str, dict[str, Any]], closes: dict[str, list[dict[str, Any]]]
) -> None:
    types = _issue_types(issues)
    closed_bugs = [i for i in closes if types[i] == _BUG_TYPE_ID]
    assert closed_bugs, "no closed bug in the whole window — bugs_fixed would read zero"


# ─── Dimensions the journal joins ────────────────────────────────────────


def test_the_issuetype_catalogue_declares_every_type_once(rows: Rows) -> None:
    seen: dict[str, str] = {}
    for row in rows["bronze_jira.jira_issuetypes"]:
        assert row["issuetype_id"] not in seen, f"type {row['issuetype_id']} is declared twice"
        seen[row["issuetype_id"]] = row["name"]

    assert seen == {type_id: name for name, (type_id, _) in task._ISSUE_TYPE_DIM.items()}


def test_the_status_catalogue_declares_a_done_category_for_every_close_status(rows: Rows) -> None:
    """Gold detects a closed task by status CATEGORY, never a localized name,
    so a close status whose row is missing or mis-categorised reads as open."""
    categories = {row["name"]: row["category_key"] for row in rows["bronze_jira.jira_statuses"]}

    for status in task._CLOSE_STATUSES:
        assert categories.get(status) == "done", (
            f"close status {status!r} is categorised {categories.get(status)!r}"
        )


def test_every_field_the_snapshot_carries_is_declared_in_the_catalogue(rows: Rows) -> None:
    """`jira__issue_field_snapshot` RAISES on a field whose kind is unknown,
    so an undeclared field fails the whole build rather than dropping a value."""
    declared = {row["field_id"] for row in rows["bronze_jira.jira_fields"]}

    assert declared >= _INITIAL_FIELDS, f"undeclared fields: {sorted(_INITIAL_FIELDS - declared)}"


def test_the_value_map_carries_a_bug_decision_for_the_snapshot_id(rows: Rows) -> None:
    """The reconciliation gold classifies an issue through: an issuetype id with
    no mapping row leaves bugs_fixed empty while every row still looks seeded."""
    decisions = {
        (row["insight_source_id"], row["field"], row["source_key"]): row["target_value"]
        for row in rows["config.field_value_map"]
    }

    assert (task.JIRA_SOURCE_ID, "issue_type", _BUG_TYPE_ID) in decisions, (
        "no issue_type decision for the bug type id"
    )
    assert decisions[(task.JIRA_SOURCE_ID, "issue_type", _BUG_TYPE_ID)] == "bug"


def test_the_source_carries_a_default_decision_for_each_classified_field(rows: Rows) -> None:
    """`assert_task_field_value_defaults_exist` blocks the gold build without
    one, so a field classified by the map needs its fallback too."""
    defaults = {
        (row["insight_source_id"], row["field"]) for row in rows["config.field_value_defaults"]
    }

    for field in ("issue_type", "resolution"):
        assert (task.JIRA_SOURCE_ID, field) in defaults, f"no default decision for {field}"


def test_every_issue_key_is_unique_and_numeric(issues: dict[str, dict[str, Any]]) -> None:
    """`jira__issue_key_ids` keeps only `^[^-]+-[0-9]+$` keys over `^[0-9]+$`
    ids, and collapses colliding keys with `GROUP BY issue_key`. A hash of the
    plan's uuid satisfied neither: four hex characters over a few hundred
    issues collide by birthday, and most are not numeric at all."""
    keys = [issue["id_readable"] for issue in issues.values()]

    assert len(set(keys)) == len(keys), "two issues share a readable key"
    assert len(set(issues)) == len(issues), "two issues share a jira_id"
    for key, jira_id in zip(keys, issues, strict=True):
        assert re.match(r"^[^-]+-[0-9]+$", key), f"{key} is not a numeric Jira key"
        assert re.match(r"^[0-9]+$", jira_id), f"{jira_id} is not a numeric Jira id"


def test_a_worklog_names_an_issue_the_run_emitted(
    rows: Rows, issues: dict[str, dict[str, Any]]
) -> None:
    """A bronze worklog names its issue by id and the staging model joins it."""
    worklogs = rows["bronze_jira.jira_worklogs"]
    assert worklogs, "no worklog was emitted"

    for row in worklogs:
        assert row["jira_id"] in issues, (
            f"worklog names issue {row['jira_id']}, which no row carries"
        )
        assert row["id_readable"] == issues[row["jira_id"]]["id_readable"], (
            f"worklog {row['worklog_id']} disagrees with the issue's readable key"
        )


@pytest.mark.parametrize("table", _TABLES)
def test_every_seeded_row_belongs_to_the_install_tenant(rows: Rows, table: str) -> None:
    """The task classes are built from these rows and joined on tenant, so a row without it reads as nobody's."""
    assert rows[table], f"{table} seeds no rows"
    strays = {row.get("tenant_id") for row in rows[table]} - {_TENANT}
    assert not strays, f"{table} carries rows outside the install tenant: {strays!r}"


def test_every_bronze_row_the_generator_writes_carries_the_install_tenant(rows: Rows) -> None:
    """The staging models refuse a bronze row without a tenant, so one such row fails the whole build."""
    bronze = {table: table_rows for table, table_rows in rows.items() if table.startswith("bronze_")}
    assert bronze, "the generator writes no bronze"

    tenantless = {
        table: len([row for row in table_rows if row.get("tenant_id") != _TENANT])
        for table, table_rows in bronze.items()
    }

    assert not any(tenantless.values()), f"bronze rows without the install tenant: {tenantless!r}"
