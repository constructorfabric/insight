"""The rules the tenant migration applies before it touches a warehouse."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from tenant_migration.apply import MigrationError, retarget
from tenant_migration.clickhouse import ClickHouse, ClickHouseError
from tenant_migration.inventory import RESTORE, Relation, Table, Unfit, classify
from tenant_migration.keys import Columns, TenantError, Tenants
from tenant_migration.plan import TablePlan, refusals

NEW = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"


@pytest.mark.parametrize(
    ("new", "old"),
    [("Example-Legacy", "example-legacy"), (NEW.upper(), "example-legacy"), (NEW, NEW), (NEW, "it's"), (NEW, "a\\b")],
    ids=["new not a uuid", "upper case", "old equals new", "quote", "backslash"],
)
def test_tenants_that_cannot_be_moved_safely_are_refused(new: str, old: str) -> None:
    with pytest.raises(TenantError):
        Tenants.parse(new, old)


def test_an_empty_tenant_always_moves_beside_the_old_one() -> None:
    assert Tenants.parse(NEW, "example-legacy").moving == ("example-legacy", "")
    assert Tenants.parse(NEW, "").moving == ("",)


STRING = "Nullable(String)"


@pytest.mark.parametrize(
    ("types", "expected"),
    [
        (
            {"unique_key": "String", "tenant_id": STRING, "source_id": STRING},
            Columns("tenant_id", "source_id", "unique_key"),
        ),
        (
            {"unique_key": "String", "insight_tenant_id": STRING, "insight_source_id": "String"},
            Columns("insight_tenant_id", "insight_source_id", "unique_key"),
        ),
        ({"tenant_id": "LowCardinality(String)", "source_id": "String"}, Columns("tenant_id", "source_id", None)),
    ],
    ids=["bronze", "silver-shaped", "no key"],
)
def test_a_table_is_read_through_its_own_column_names(types: dict[str, str], expected: Columns) -> None:
    table = classify("db", "t", "ReplacingMergeTree", types)

    assert isinstance(table, Table), table
    assert table.columns == expected


@pytest.mark.parametrize(
    "types",
    [
        {"unique_key": "String", "source_id": STRING},
        {"unique_key": "String", "tenant_id": "UUID", "source_id": STRING},
        {"unique_key": "String", "tenant_id": STRING},
    ],
    ids=["no tenant", "tenant not a string", "key without a source"],
)
def test_a_table_the_migration_cannot_read_is_unfit(types: dict[str, str]) -> None:
    assert isinstance(classify("db", "t", "MergeTree", types), Unfit), f"should be unfit: {types}"


def _plan(**overrides: object) -> TablePlan:
    table = Table("bronze_example", "items", "ReplacingMergeTree", Columns("tenant_id", "source_id", "unique_key"))
    fields = {"table": table, "rows": 10, "moving": 10, "ambiguous": 0, "foreign": (), "leftovers": ()}
    return TablePlan(**(fields | overrides))  # type: ignore[arg-type]


@pytest.mark.parametrize(
    ("plan", "reason"),
    [
        (_plan(foreign=("other",)), "holds another tenant"),
        (_plan(ambiguous=1), "start like a moving tenant"),
        (_plan(leftovers=("bronze_example.items__tenant_backup",)), "finalize or roll back first"),
    ],
    ids=["second tenant", "ambiguous key", "earlier run"],
)
def test_a_plan_is_refused_for_every_reason_it_cannot_be_applied(plan: TablePlan, reason: str) -> None:
    found = refusals([plan], [])

    assert len(found) == 1 and reason in found[0], found


def test_work_left_by_an_earlier_run_is_refused_even_where_nothing_moves() -> None:
    """A finished run waits for finalize or rollback; a new one must not start beside it."""
    found = refusals([_plan(moving=0, leftovers=("bronze_example.items__tenant_backup",))], [])

    assert len(found) == 1 and "finalize or roll back first" in found[0], found


def test_an_unfit_table_with_rows_is_refused_and_an_empty_one_is_not() -> None:
    unfit = Unfit("bronze_example.legacy", "no tenant column")

    assert refusals([], [(unfit, 3)]) == ["bronze_example.legacy: 3 row(s), and no tenant column"]
    assert refusals([], []) == []


def test_an_unreachable_clickhouse_is_reported_as_an_error_not_a_traceback() -> None:
    unreachable = ClickHouse(url="http://127.0.0.1:1/", user="insight", password="insight")

    with pytest.raises(ClickHouseError, match="unreachable"):
        unreachable.execute("SELECT 1")


@pytest.mark.parametrize(
    "saved",
    [
        "CREATE TABLE bronze_example.items\n(`unique_key` String)\nENGINE = MergeTree ORDER BY unique_key",
        "CREATE TABLE bronze_example.items UUID '5f1c0e3a-7b2d-4c8e-9a01-23456789abcd'\n(`unique_key` String)\n"
        "ENGINE = MergeTree ORDER BY unique_key",
    ],
    ids=["without uuid", "with the dumped table's uuid"],
)
def test_the_saved_ddl_names_the_restore_table_and_no_uuid(saved: str) -> None:
    rel = Relation("bronze_example", "items")

    created = retarget(saved, rel, RESTORE)

    assert created.startswith(f"CREATE TABLE {rel.sql(RESTORE)}\n(`unique_key` String)"), created
    assert "UUID" not in created, f"should drop the dumped table's uuid: {created!r}"


def test_saved_ddl_that_is_not_a_create_table_is_refused() -> None:
    with pytest.raises(MigrationError):
        retarget("ATTACH TABLE bronze_example.items", Relation("bronze_example", "items"), RESTORE)

