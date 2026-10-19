"""A staging relation that is there is not the same as a dependency that is met.

A staging model is a VIEW: reading it re-runs its query. So one left behind by an
earlier run, over bronze that has since gone, fails whoever unions it. Two rules
follow, and both are asserted here:

  - a class whose every arm is stale leaves the run with them;
  - a class with ONE healthy arm stays, and the union leaves the stale arm out.

The second is the one that bites: excluding the arm from the run does not stop
`union_by_tag` reading it, so the union has to make the same judgement.

The stale views are built by hand over tables that are then dropped, because the
state under test is a view whose source is absent — the decision reads the
relation's existence and the model's materialization, never the view's columns,
so no connector's bronze shape belongs in this test.
"""

from __future__ import annotations

import pytest
from conftest import Warehouse

#: Fed only by arms that need bronze, so every one of them goes stale.
ORPHANED_CLASS = "class_people"
ORPHANED_ARM = "active_directory__to_class_people"

#: Fed by a GitHub arm that reads no bronze at all, so it survives its stale peer.
SURVIVING_CLASS = "class_task_statuses"
STALE_PEER = "jira__task_statuses"
HEALTHY_ARM = "github__task_statuses"


@pytest.fixture(scope="module")
def stale_arms_left_behind(fresh_install: Warehouse) -> Warehouse:
    """A warehouse holding two staging views whose bronze has gone.

    INVARIANT: the bronze goes whether or not the views were built. A fixture
    that raised in between would leave a connector's bronze standing for
    whatever runs next, and the next run's cold start would not be one.
    """
    # The created names are spelled out: ClickHouse takes a bound identifier
    # where a statement READS one, but not as the object a CREATE VIEW names.
    # They are the models ORPHANED_ARM and STALE_PEER name.
    try:
        _stale_view(
            fresh_install,
            bronze="bronze_active_directory",
            statement="CREATE VIEW staging.active_directory__to_class_people "
            "AS SELECT unique_key FROM {database:Identifier}.gone",
        )
        _stale_view(
            fresh_install,
            bronze="bronze_jira",
            statement="CREATE VIEW staging.jira__task_statuses AS SELECT unique_key FROM {database:Identifier}.gone",
        )
    finally:
        for bronze in ("bronze_active_directory", "bronze_jira"):
            fresh_install.execute("DROP DATABASE IF EXISTS {database:Identifier}", {"database": bronze})

    return fresh_install


def _stale_view(warehouse: Warehouse, *, bronze: str, statement: str) -> None:
    """Build a staging view over a bronze table, for a caller that drops it."""
    warehouse.execute("CREATE DATABASE IF NOT EXISTS {database:Identifier}", {"database": bronze})
    warehouse.execute(
        "CREATE TABLE {database:Identifier}.gone (unique_key String) ENGINE = MergeTree ORDER BY unique_key",
        {"database": bronze},
    )
    warehouse.execute(statement, {"database": bronze})


def test_a_staging_view_over_absent_bronze_is_not_buildable(stale_arms_left_behind: Warehouse) -> None:
    unbuildable = stale_arms_left_behind.unbuildable_models()

    assert ORPHANED_ARM in unbuildable
    assert STALE_PEER in unbuildable


def test_a_class_whose_every_arm_is_stale_leaves_the_run(stale_arms_left_behind: Warehouse) -> None:
    assert ORPHANED_CLASS in stale_arms_left_behind.unbuildable_models(), (
        "a class whose only materialised arm re-runs over absent bronze would fail the build it was kept in"
    )


def test_one_healthy_arm_keeps_its_class_in_the_run(stale_arms_left_behind: Warehouse) -> None:
    unbuildable = stale_arms_left_behind.unbuildable_models()

    assert HEALTHY_ARM not in unbuildable
    assert SURVIVING_CLASS not in unbuildable


def test_the_union_leaves_out_the_stale_arm_its_class_survived(stale_arms_left_behind: Warehouse) -> None:
    # The proof the exclusion alone cannot give: the class IS built, so
    # `union_by_tag` had to judge the stale arm for itself.
    completed = stale_arms_left_behind.run_dbt_sh()

    assert completed.returncode == 0, completed.stdout[-8000:]
    assert f"skipping {STALE_PEER}" in completed.stdout, (
        f"the union must say it left the stale arm out: {completed.stdout[-4000:]}"
    )
