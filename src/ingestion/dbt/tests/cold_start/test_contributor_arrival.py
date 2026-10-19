"""One contributor is enough.

A silver class is fed by one staging arm per source, and `union_by_tag` unions
whichever of them are materialised. So the rule in front of it is an OR, not an
AND: the class becomes buildable the moment ANY of its arms can be built, and
the arms still waiting on their connector stay out of the union and out of the
run.

The bronze tables here are empty stubs rather than a connector's real output.
Only their existence is read — `list_unbuildable_models` probes whether a
relation is there, never what shape it has — and the suites that care about the
shape create bronze through the destination, the one creator a deployment has.
"""

from __future__ import annotations

import pytest
from conftest import Warehouse

#: A class with one arm per HR source, of which only this one is given bronze.
CLASS = "class_people"
ARRIVED = "active_directory__to_class_people"
STILL_ABSENT = ("bamboohr__to_class_people", "ms_entra__to_class_people")
ARRIVED_BRONZE = "bronze_active_directory"


@pytest.fixture(scope="module")
def after_arrival(fresh_install: Warehouse) -> tuple[set[str], set[str]]:
    """What the check reports before and after one connector's bronze lands."""
    before = fresh_install.unbuildable_models()

    fresh_install.execute("CREATE DATABASE IF NOT EXISTS {database:Identifier}", {"database": ARRIVED_BRONZE})
    tables = fresh_install.source_tables(ARRIVED_BRONZE)
    assert tables, f"the project declares no sources in {ARRIVED_BRONZE}; this test names a connector that moved"
    for table in sorted(tables):
        fresh_install.execute(
            "CREATE TABLE IF NOT EXISTS {database:Identifier}.{name:Identifier} "
            "(unique_key String) ENGINE = MergeTree ORDER BY unique_key",
            {"database": ARRIVED_BRONZE, "name": table},
        )

    return before, fresh_install.unbuildable_models()


def test_a_class_waits_while_every_arm_waits(after_arrival: tuple[set[str], set[str]]) -> None:
    before, _ = after_arrival
    assert CLASS in before
    assert ARRIVED in before


def test_one_arms_bronze_makes_its_class_buildable(after_arrival: tuple[set[str], set[str]]) -> None:
    _, after = after_arrival
    assert ARRIVED not in after
    assert CLASS not in after


def test_the_arms_still_without_bronze_stay_out_of_the_run(after_arrival: tuple[set[str], set[str]]) -> None:
    _, after = after_arrival
    assert set(STILL_ABSENT) <= after
