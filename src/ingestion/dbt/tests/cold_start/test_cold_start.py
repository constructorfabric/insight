"""A dbt run against a warehouse whose connectors have never synced.

Silver is a projection of bronze, so a class nobody feeds has no honest shape to
emit and is not built at all. `union_by_tag` already unions whichever tagged
contributors are materialised; what these tests cover is the decision in front
of it, because dbt fixes its node set before the first model runs and a model
body cannot skip itself. `list_unbuildable_models` makes that decision and
`run-dbt.sh` acts on it.

Nothing here seeds a connector: the subject is which models a run builds, never
how many rows they end up with.
"""

from __future__ import annotations

from dataclasses import dataclass

import pytest
from conftest import Warehouse

#: Every silver class relation is named for its class contract.
CLASS_PREFIX = "class_"


@dataclass(frozen=True)
class ColdRun:
    """One `run-dbt.sh` against a warehouse that holds no bronze."""

    returncode: int
    output: str
    unbuildable: frozenset[str]
    relations: frozenset[str]

    def silver_classes(self) -> set[str]:
        return {name.removeprefix("silver.") for name in self.relations if name.startswith(f"silver.{CLASS_PREFIX}")}


@pytest.fixture(scope="module")
def cold_run(fresh_install: Warehouse) -> ColdRun:
    unbuildable = frozenset(fresh_install.unbuildable_models())
    completed = fresh_install.run_dbt_sh()
    return ColdRun(
        returncode=completed.returncode,
        output=f"{completed.stdout[-8000:]}\n{completed.stderr[-4000:]}",
        unbuildable=unbuildable,
        relations=frozenset(fresh_install.relations()),
    )


def test_a_run_against_a_warehouse_with_no_bronze_succeeds(cold_run: ColdRun) -> None:
    assert cold_run.returncode == 0, cold_run.output


def test_a_model_reported_unbuildable_is_never_built(cold_run: ColdRun) -> None:
    built = {name.split(".", 1)[1] for name in cold_run.relations}
    assert not built & cold_run.unbuildable, f"should not have been built: {sorted(built & cold_run.unbuildable)}"


def test_a_class_whose_contributor_needs_no_bronze_is_built_and_empty(
    cold_run: ColdRun, fresh_install: Warehouse
) -> None:
    built = cold_run.silver_classes()
    assert built, "no silver class is reachable without bronze, so this lane proves nothing about the honest path"
    for relation in sorted(built):
        assert fresh_install.rows_in("silver", relation) == 0, f"should be empty with no bronze: {relation}"


def test_a_class_nobody_feeds_is_not_built_at_all(cold_run: ColdRun) -> None:
    unfed = {name for name in cold_run.unbuildable if name.startswith(CLASS_PREFIX)}
    assert unfed, "every silver class is buildable from nothing, so the skip is untested"
    assert not unfed & cold_run.silver_classes()


def test_a_second_run_over_the_same_warehouse_changes_nothing(cold_run: ColdRun, fresh_install: Warehouse) -> None:
    repeated = fresh_install.run_dbt_sh()

    assert repeated.returncode == 0, repeated.stdout[-8000:]
    assert frozenset(fresh_install.relations()) == cold_run.relations
