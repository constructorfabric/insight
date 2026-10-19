"""A run removes the staging relations an interrupted incremental run left behind.

dbt-clickhouse stages a `delete+insert` run's rows in
`<model>__dbt_new_data_<invocation id>` and drops it last; a run killed before
that leaves it in `staging` for good. The `on-run-start` hook drops such a
relation once it is older than `incremental_intermediate_orphan_min_age_hours`,
and leaves alone everything that is not one: a younger one, which a concurrent
run may still be writing, a lookalike of no incremental model, and the journal.
"""

from __future__ import annotations

from typing import Any

from conftest import Scenario
from helpers import SOURCE_ID, event, field, issue, item

STORY_POINTS = "customfield_10001"

ORPHAN = "jira__field_history_derived__dbt_new_data_0a1b2c3d_0000_4000_8000_000000000001"
LOOKALIKE = "not_a_dbt_model__dbt_new_data_0a1b2c3d_0000_4000_8000_000000000002"

EVERY_RELATION_IS_OLD_ENOUGH = {"incremental_intermediate_orphan_min_age_hours": 0}


def _seed(scenario: Scenario) -> None:
    scenario.seed(
        fields=[field(STORY_POINTS, name="Story Points", schema_type="number")],
        issues=[issue("TST-1", fields={STORY_POINTS: 5})],
        events=[
            event("TST-1", 101, "2026-01-06T10:00:00", [item(STORY_POINTS, frm="3", frm_str="3", to="5", to_str="5")])
        ],
    )


def _journal(scenario: Scenario) -> list[dict[str, Any]]:
    return scenario.warehouse.rows(
        "SELECT unique_key, value_ids, _version"
        " FROM staging.jira__field_history_derived FINAL"
        " WHERE insight_source_id = {src:String}"
        " ORDER BY unique_key",
        {"src": SOURCE_ID},
    )


def _staging_tables(scenario: Scenario) -> set[str]:
    rows = scenario.warehouse.rows("SELECT name FROM system.tables WHERE database = 'staging'")
    return {r["name"] for r in rows}


def _plant_orphans(scenario: Scenario) -> None:
    # A table name cannot be bound as a parameter, so each statement is spelled out.
    scenario.warehouse.execute(
        "CREATE TABLE staging.jira__field_history_derived__dbt_new_data_0a1b2c3d_0000_4000_8000_000000000001"
        " AS staging.jira__field_history_derived"
    )
    scenario.warehouse.execute(
        "INSERT INTO staging.jira__field_history_derived__dbt_new_data_0a1b2c3d_0000_4000_8000_000000000001"
        " SELECT * FROM staging.jira__field_history_derived"
    )
    scenario.warehouse.execute(
        "CREATE TABLE staging.not_a_dbt_model__dbt_new_data_0a1b2c3d_0000_4000_8000_000000000002"
        " (x UInt8) ENGINE = MergeTree ORDER BY x"
    )


def test_a_run_drops_an_abandoned_intermediate_and_keeps_the_journal(scenario: Scenario) -> None:
    _seed(scenario)
    scenario.build()
    journal_before = _journal(scenario)
    _plant_orphans(scenario)

    try:
        scenario.build(full_refresh=False)
        assert {ORPHAN, LOOKALIKE} <= _staging_tables(scenario), "a relation younger than the threshold must be kept"

        scenario.build(full_refresh=False, dbt_vars=EVERY_RELATION_IS_OLD_ENOUGH)
        tables = _staging_tables(scenario)

        assert ORPHAN not in tables, "an abandoned intermediate of an incremental model must be dropped"
        assert LOOKALIKE in tables, "a name that belongs to no incremental model must be kept"
        assert not [t for t in tables if "__dbt_new_data_" in t and t != LOOKALIKE], (
            f"the runs must leave no intermediate of their own: {sorted(tables)}"
        )
        assert _journal(scenario) == journal_before, "the live journal must be left as it was"
    finally:
        scenario.warehouse.execute(
            "DROP TABLE IF EXISTS staging.jira__field_history_derived__dbt_new_data_0a1b2c3d_0000_4000_8000_000000000001"
        )
        scenario.warehouse.execute(
            "DROP TABLE IF EXISTS staging.not_a_dbt_model__dbt_new_data_0a1b2c3d_0000_4000_8000_000000000002"
        )
