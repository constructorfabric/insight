"""A scope derived in batches lands exactly where one statement lands.

A run whose scope exceeds `jira_journal_issues_per_batch` derives it one batch
of issues per statement: the model's own statement takes batch 0 and the
post-hook replays it for the rest. An issue is the unit of recomputation, so the
split may change the memory a run needs and nothing else. Each test builds the
same bronze unbatched and batched, and compares the journals row by row.

One issue per batch is the harshest split: every issue is derived alone, and
some batches hold none.
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

ONE_ISSUE_PER_BATCH = {"jira_journal_issues_per_batch": 1}

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


def _batches_run(scenario: Scenario, since: str) -> set[int]:
    """The batch numbers the statements since `since` derived."""
    scenario.warehouse.execute("SYSTEM FLUSH LOGS")
    rows = scenario.warehouse.rows(
        "SELECT DISTINCT toUInt32(arrayJoin(extractAll(query, 'jira-journal batch ([0-9]+) of'))) AS batch"
        " FROM system.query_log"
        " WHERE type = 'QueryFinish' AND event_time_microseconds >= parseDateTime64BestEffort({since:String}, 6)",
        {"since": since},
    )
    return {r["batch"] for r in rows}


def _now(scenario: Scenario) -> str:
    return scenario.warehouse.rows("SELECT toString(now64(6)) AS now")[0]["now"]


def _seed_four_issues(scenario: Scenario) -> None:
    scenario.seed(
        fields=FIELDS,
        issues=[
            issue("TST-1", jira_id="1001", fields={STORY_POINTS: 5, LABELS: ["a", "b"]}),
            issue("TST-2", jira_id="1002", fields={STORY_POINTS: 8, COMPONENTS: [{"id": "10", "name": "core"}]}),
            issue("TST-3", jira_id="1003", fields={STORY_POINTS: 2}),
            issue("TST-4", jira_id="1004", fields={COMPONENTS: [{"id": "11", "name": "ui"}]}),
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
            event(
                "TST-4",
                401,
                "2026-01-08T10:00:00",
                [item(COMPONENTS, frm="10", frm_str="core", to="11", to_str="ui")],
                jira_id="1004",
            ),
        ],
    )


def test_a_batched_full_refresh_equals_one_statement(scenario: Scenario) -> None:
    _seed_four_issues(scenario)
    scenario.build()
    whole = _journal(scenario)
    assert {v[0] for v in whole.values()} == {"TST-1", "TST-2", "TST-3", "TST-4"}

    since = _now(scenario)
    scenario.build(dbt_vars=ONE_ISSUE_PER_BATCH)

    assert len(_batches_run(scenario, since)) > 1, "the run was not split"
    assert _journal(scenario) == whole
    assert scenario.invariants_hold()


def test_a_batched_rebuild_replaces_every_issue_it_does_not_derive_in_the_first_batch(scenario: Scenario) -> None:
    """A rebuild without `--full-refresh` keeps the table, so every batch the
    post-hook derives must also remove that batch's old rows — including a row
    no current derivation produces."""
    _seed_four_issues(scenario)
    scenario.build()
    whole = _journal(scenario)

    scenario.warehouse.execute("ALTER TABLE staging.jira__field_history_derived MODIFY COMMENT ''")
    # A stale row on every issue, under a key no derivation emits.
    scenario.warehouse.execute(
        "INSERT INTO staging.jira__field_history_derived"
        " SELECT * REPLACE (concat(unique_key, '-stale') AS unique_key)"
        " FROM staging.jira__field_history_derived FINAL WHERE event_kind = 'synthetic_initial' AND field_id = 'created'"
    )

    since = _now(scenario)
    scenario.build(full_refresh=False, dbt_vars=ONE_ISSUE_PER_BATCH)

    assert len(_batches_run(scenario, since)) > 1, "the run was not split"
    assert _journal(scenario) == whole, "a batch left an old row of its issues behind"
    assert scenario.warehouse.rows(
        "SELECT comment FROM system.tables WHERE database = 'staging' AND name = 'jira__field_history_derived'"
    )[0]["comment"].startswith("jira-journal-catalogue fingerprint=")


def test_a_batched_incremental_run_lands_where_a_rebuild_lands(scenario: Scenario) -> None:
    """Touched issues split into batches too: a sync that re-delivers most of
    bronze must not need the memory of a rebuild in one statement."""
    _seed_four_issues(scenario)
    scenario.build()

    scenario.warehouse.insert(
        "bronze_jira.jira_issue",
        [
            issue("TST-1", jira_id="1001", fields={STORY_POINTS: 5, LABELS: ["b"]}, extracted_at=LATER_SYNC),
            issue("TST-3", jira_id="1003", fields={STORY_POINTS: 3}, extracted_at=LATER_SYNC),
        ],
    )
    scenario.warehouse.insert(
        "bronze_jira.jira_issue_history",
        [
            event(
                "TST-1",
                102,
                "2026-01-09T10:00:00",
                [item(LABELS, frm_str="a b", to_str="b")],
                jira_id="1001",
                extracted_at=LATER_SYNC,
            ),
            event(
                "TST-3",
                301,
                "2026-01-09T11:00:00",
                [item(STORY_POINTS, frm="2", frm_str="2", to="3", to_str="3")],
                jira_id="1003",
                extracted_at=LATER_SYNC,
            ),
        ],
    )
    since = _now(scenario)
    scenario.build(full_refresh=False, dbt_vars=ONE_ISSUE_PER_BATCH)
    rolled_forward = _journal(scenario)

    assert len(_batches_run(scenario, since)) > 1, "the run was not split"
    assert scenario.states(LABELS, issue="TST-1") == [["a"], ["a", "b"], ["b"]]

    # `_version` aside, as in `test_rolling_forward_lands_where_a_full_rebuild_of_the_same_bronze_lands`.
    scenario.build()
    assert {k: v[:-1] for k, v in _journal(scenario).items()} == {k: v[:-1] for k, v in rolled_forward.items()}
    assert scenario.invariants_hold()
