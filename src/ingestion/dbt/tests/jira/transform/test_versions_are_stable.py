"""A rebuild over unchanged bronze reproduces the same `_version` per row.

The journal is a table, recomputed in full on every run; the class that unions
it is incremental and admits rows above its newest version. Were `_version` the
build time, every run would hand the class the entire Jira journal again and it
would delete and re-insert all of it. So the version is the issue's input
freshness, and this test holds that in place: nothing changes, nothing moves;
one issue receives an entry, only that issue's rows move.
"""

from __future__ import annotations

from conftest import Scenario
from helpers import LATER_SYNC, SOURCE_ID, event, field, issue, item

STORY_POINTS = "customfield_10001"


def _versions(scenario: Scenario) -> dict[str, tuple[str, int]]:
    """unique_key -> (issue key, _version) for every journal row."""
    rows = scenario.warehouse.rows(
        "SELECT unique_key, id_readable, _version"
        " FROM staging.jira__field_history_derived FINAL"
        " WHERE insight_source_id = {src:String}",
        {"src": SOURCE_ID},
    )
    return {r["unique_key"]: (r["id_readable"], r["_version"]) for r in rows}


def test_a_rebuild_over_unchanged_bronze_keeps_every_version(scenario: Scenario) -> None:
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: 5}), issue("TST-2", fields={STORY_POINTS: 8})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [item(STORY_POINTS, frm="3", frm_str="3", to="5", to_str="5")])
        ],
    )
    scenario.build()
    first = _versions(scenario)
    assert first, "the scenario produced no journal"

    scenario.build()
    assert _versions(scenario) == first


def test_only_the_issue_that_received_an_entry_moves(scenario: Scenario) -> None:
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: 5}), issue("TST-2", fields={STORY_POINTS: 8})],
    )
    scenario.build()
    before = _versions(scenario)

    scenario.warehouse.insert(
        "bronze_jira.jira_issue_history",
        [
            event(
                "TST-2",
                201,
                "2026-01-07T10:00:00",
                [item(STORY_POINTS, frm="3", frm_str="3", to="8", to_str="8")],
                extracted_at=LATER_SYNC,
            )
        ],
    )
    scenario.build()
    after = _versions(scenario)

    untouched = {k: v for k, v in before.items() if v[0] == "TST-1"}
    assert {k: v for k, v in after.items() if v[0] == "TST-1"} == untouched
    moved = {k: v for k, v in after.items() if v[0] == "TST-2"}
    assert moved, "TST-2 has no rows"
    assert all(v[1] > before[k][1] for k, v in moved.items() if k in before), (
        "a row of the issue that received an entry kept its old version"
    )
