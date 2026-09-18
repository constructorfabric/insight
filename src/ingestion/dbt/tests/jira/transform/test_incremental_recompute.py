"""A run without `--full-refresh` recomputes exactly the issues bronze touched.

The journal is incremental at the ISSUE grain: an issue that received a new
row or changelog entry since the last build has its whole journal derived again
and every row it held replaced; every other issue is left as it was. A field
catalogue that differs from the one the last build used rebuilds every issue,
because rows that depend on it belong to issues bronze never touched.

Each test builds once in full, changes bronze, builds again WITHOUT the flag,
and compares the two journals row by row.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario
from helpers import LATER_SYNC, SOURCE_ID, event, field, issue, item

STORY_POINTS = "customfield_10001"
LABELS = "labels"
COMPONENTS = "components"

FIELDS = [
    field(STORY_POINTS, name="Story Points", schema_type="number"),
    field(LABELS, name="Labels", schema_type="array", schema_items="string"),
    field(COMPONENTS, name="Components", schema_type="array", schema_items="component"),
]

# Rows as (unique_key -> the columns a reader depends on), so two journals can
# be compared exactly and a difference names the row.
Journal = dict[str, tuple[Any, ...]]


def _journal(scenario: Scenario) -> Journal:
    rows = scenario.warehouse.rows(
        "SELECT unique_key, id_readable, event_kind, event_id, toString(event_at) AS event_at,"
        "       _seq, delta_action, value_ids, value_displays, _version"
        " FROM staging.jira__field_history_derived FINAL"
        " WHERE insight_source_id = {src:String}",
        {"src": SOURCE_ID},
    )
    return {
        r["unique_key"]: (
            r["id_readable"],
            r["event_kind"],
            r["event_id"],
            r["event_at"],
            r["_seq"],
            r["delta_action"],
            r["value_ids"],
            r["value_displays"],
            r["_version"],
        )
        for r in rows
    }


def _of_issue(journal: Journal, issue_id: str) -> Journal:
    return {k: v for k, v in journal.items() if k.startswith(f"{SOURCE_ID}-jira-{issue_id}-")}


def _seed_two_issues(scenario: Scenario) -> None:
    scenario.seed(
        fields=FIELDS,
        issues=[
            issue("TST-1", jira_id="1001", fields={STORY_POINTS: 5, LABELS: ["a", "b"]}),
            issue("TST-2", jira_id="1002", fields={STORY_POINTS: 8, COMPONENTS: [{"id": "10", "name": "core"}]}),
        ],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(STORY_POINTS, frm="3", frm_str="3", to="5", to_str="5"), item(LABELS, frm_str="a", to_str="a b")],
                jira_id="1001",
            ),
            event("TST-2", 201, "2026-01-07T10:00:00", [item(COMPONENTS, to="10", to_str="core")], jira_id="1002"),
        ],
    )


def test_an_incremental_run_over_unchanged_bronze_changes_nothing(scenario: Scenario) -> None:
    _seed_two_issues(scenario)
    scenario.build()
    before = _journal(scenario)
    assert before, "the scenario produced no journal"

    scenario.build(full_refresh=False)

    assert _journal(scenario) == before
    assert scenario.invariants_hold()


def test_only_the_touched_issue_is_recomputed_and_its_vanished_item_leaves_no_row(scenario: Scenario) -> None:
    """A changelog entry re-emitted by a later sync replaces the earlier
    emission of the same id. An item the new emission no longer carries must
    leave no row behind — an append could not remove it — and an issue whose
    inputs did not move keeps every row and every version."""
    _seed_two_issues(scenario)
    scenario.build()
    before = _journal(scenario)

    # The issue as the later sync sees it: labels down to `b`, so the round trip
    # has a current value to land on.
    scenario.warehouse.insert(
        "bronze_jira.jira_issue",
        [issue("TST-1", jira_id="1001", fields={STORY_POINTS: 5, LABELS: ["b"]}, extracted_at=LATER_SYNC)],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_history",
        [
            # Entry 101 again, later, without the story-points item.
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(LABELS, frm_str="a", to_str="a b")],
                jira_id="1001",
                extracted_at=LATER_SYNC,
            ),
            event(
                "TST-1",
                102,
                "2026-01-08T10:00:00",
                [item(LABELS, frm_str="a b", to_str="b")],
                jira_id="1001",
                extracted_at=LATER_SYNC,
            ),
        ],
    )
    scenario.build(full_refresh=False)
    after = _journal(scenario)

    assert _of_issue(after, "1002") == _of_issue(before, "1002"), "an untouched issue moved"

    touched = _of_issue(after, "1001")
    story_points_rows = {k: v for k, v in touched.items() if f"-{STORY_POINTS}-" in k}
    assert [v[1] for v in story_points_rows.values()] == ["synthetic_initial"], (
        "the item entry 101 no longer carries must not survive as a changelog row"
    )
    assert scenario.states(LABELS, issue="TST-1") == [["a"], ["a", "b"], ["b"]]
    assert all(v[-1] > before[k][-1] for k, v in touched.items() if k in before), (
        "a row of the touched issue kept its old version"
    )
    assert scenario.invariants_hold()


def test_a_move_after_the_first_build_leaves_one_history_under_the_new_key(scenario: Scenario) -> None:
    """Jira renames an issue when it moves and keeps its id. The rows the first
    build keyed by that id are the ones the second replaces, so the old key
    survives nowhere — not as a second issue, not as a stale row."""
    _seed_two_issues(scenario)
    scenario.build()

    scenario.warehouse.insert(
        "bronze_jira.jira_issue",
        [issue("NEW-9", jira_id="1001", fields={STORY_POINTS: 13, LABELS: ["a", "b"]}, extracted_at=LATER_SYNC)],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_history",
        [
            event(
                "NEW-9",
                103,
                "2026-02-01T10:00:00",
                [item(STORY_POINTS, frm="5", frm_str="5", to="13", to_str="13")],
                jira_id="1001",
                extracted_at=LATER_SYNC,
            )
        ],
    )
    scenario.build(full_refresh=False)

    assert scenario.journal(issue="TST-1") == [], "the old key must not survive"
    moved = _of_issue(_journal(scenario), "1001")
    assert moved and {v[0] for v in moved.values()} == {"NEW-9"}, moved
    assert scenario.states(STORY_POINTS, issue="NEW-9") == [["3"], ["5"], ["13"]]
    assert scenario.invariants_hold()


def test_a_catalogue_shift_recomputes_issues_bronze_did_not_touch(scenario: Scenario) -> None:
    """An event on a field the catalogue lacks yields one best-effort row. When
    the field's metadata arrives, that issue's history becomes derivable — and
    nothing about the issue itself moved in bronze, so only a rebuild that
    notices the catalogue can reach it."""
    late_field = "customfield_20001"
    scenario.seed(
        fields=FIELDS,
        issues=[issue("TST-1", jira_id="1001", fields={STORY_POINTS: 5, late_field: {"id": "7", "value": "High"}})],
        events=[
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(late_field, frm="6", frm_str="Low", to="7", to_str="High")],
                jira_id="1001",
            )
        ],
    )
    scenario.build()
    before = _journal(scenario)
    assert [v[1] for k, v in before.items() if f"-{late_field}-" in k] == ["unclassified_field"]

    scenario.warehouse.insert(
        "bronze_jira.jira_fields", [field(late_field, name="Severity", schema_type="option", extracted_at=LATER_SYNC)]
    )
    scenario.build(full_refresh=False)
    after = _journal(scenario)

    assert sorted(v[1] for k, v in after.items() if f"-{late_field}-" in k) == ["changelog", "synthetic_initial"]
    assert scenario.states(late_field, issue="TST-1") == [["6"], ["7"]]
    assert all(v[-1] > before[k][-1] for k, v in after.items() if k in before), (
        "a rebuild forced by the catalogue must move every version, or the class never sees it"
    )
    assert scenario.invariants_hold()


def test_versions_stay_put_across_incremental_runs_when_the_catalogue_is_stable(scenario: Scenario) -> None:
    """The floor `_version` carries is the catalogue epoch of the last full
    rebuild. Two incremental runs with the catalogue unchanged reproduce it, so
    the class downstream is handed nothing it already holds."""
    _seed_two_issues(scenario)
    scenario.build()
    before = _journal(scenario)

    scenario.build(full_refresh=False)
    scenario.build(full_refresh=False)

    assert _journal(scenario) == before
    assert scenario.warehouse.rows(
        "SELECT comment FROM system.tables WHERE database = 'staging' AND name = 'jira__field_history_derived'"
    )[0]["comment"].startswith("jira-journal-catalogue fingerprint=")


def test_a_table_carrying_no_catalogue_record_is_rebuilt_not_patched(scenario: Scenario) -> None:
    """The shape every deployed warehouse is in the first time this runs.

    A journal built before the record existed carries none, and its rows may
    include ones no issue in bronze accounts for. An issue-scoped run would
    leave those in place forever: nothing names their issue, so nothing deletes
    them. A run that finds no record must rebuild the table instead.
    """
    _seed_two_issues(scenario)
    scenario.build()
    before = _journal(scenario)

    scenario.warehouse.execute("ALTER TABLE staging.jira__field_history_derived MODIFY COMMENT ''")
    # A row of an issue bronze does not have: only a rebuild can remove it.
    scenario.warehouse.execute(
        "INSERT INTO staging.jira__field_history_derived"
        " SELECT * REPLACE ('4004' AS issue_id,"
        "        concat(insight_source_id, '-jira-4004-', field_id, '-', event_id) AS unique_key)"
        " FROM staging.jira__field_history_derived LIMIT 1"
    )
    assert _of_issue(_journal(scenario), "4004"), "the fixture row was not inserted"

    scenario.build(full_refresh=False)

    after = _journal(scenario)
    assert _of_issue(after, "4004") == {}, "a table with no catalogue record was patched, not rebuilt"
    assert after == before
    assert scenario.warehouse.rows(
        "SELECT comment FROM system.tables WHERE database = 'staging' AND name = 'jira__field_history_derived'"
    )[0]["comment"].startswith("jira-journal-catalogue fingerprint=")
    assert scenario.invariants_hold()


def test_rolling_forward_lands_where_a_full_rebuild_of_the_same_bronze_lands(scenario: Scenario) -> None:
    """The property the whole design rests on, asserted directly.

    Build at one point in the history, let bronze grow twice, follow it
    incrementally, and the journal must be the one a rebuild over that same
    bronze produces — row for row, version for version. Every other scenario
    here checks a hand-written expectation, which says the models agree with
    what someone wrote down; this says the two paths agree with each other.

    Two columns stand outside the comparison. `collected_at` stamps when a row
    was derived, and under incrementality an untouched issue keeps the stamp of
    the build that last derived it; nothing reads it from the class. `_version`
    is a publication version, and a rebuild deliberately raises it so rows it
    changed for issues bronze never touched reach the class — so the rule for
    it is that a rebuild never lowers one, asserted below.
    """
    _seed_two_issues(scenario)
    scenario.build()

    # First step: an entry re-emitted without one of its items, a second entry,
    # and the issue as the later sync sees it.
    scenario.warehouse.insert(
        "bronze_jira.jira_issue",
        [issue("TST-1", jira_id="1001", fields={STORY_POINTS: 5, LABELS: ["b"]}, extracted_at=LATER_SYNC)],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_history",
        [
            event(
                "TST-1",
                101,
                "2026-01-06T10:00:00",
                [item(LABELS, frm_str="a", to_str="a b")],
                jira_id="1001",
                extracted_at=LATER_SYNC,
            ),
            event(
                "TST-1",
                102,
                "2026-01-08T10:00:00",
                [item(LABELS, frm_str="a b", to_str="b")],
                jira_id="1001",
                extracted_at=LATER_SYNC,
            ),
        ],
    )
    scenario.build(full_refresh=False)

    # Second step: an issue that did not exist at the first build, and an
    # element-wise field losing an element on one that did.
    scenario.warehouse.insert(
        "bronze_jira.jira_issue",
        [
            issue("TST-3", jira_id="1003", fields={STORY_POINTS: 2}, extracted_at=LATER_SYNC),
            issue("TST-2", jira_id="1002", fields={STORY_POINTS: 8}, extracted_at=LATER_SYNC),
        ],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_history",
        [
            event(
                "TST-2",
                202,
                "2026-01-09T10:00:00",
                [item(COMPONENTS, frm="10", frm_str="core")],
                jira_id="1002",
                extracted_at=LATER_SYNC,
            )
        ],
    )
    scenario.build(full_refresh=False)
    rolled_forward = _journal(scenario)

    # Two empty journals would compare equal and prove nothing.
    assert {v[0] for v in rolled_forward.values()} == {"TST-1", "TST-2", "TST-3"}

    scenario.build()
    rebuilt = _journal(scenario)

    assert {k: v[:-1] for k, v in rolled_forward.items()} == {k: v[:-1] for k, v in rebuilt.items()}
    assert all(rebuilt[k][-1] >= v[-1] for k, v in rolled_forward.items()), (
        "a rebuild lowered a row's publication version"
    )
    assert scenario.invariants_hold()
