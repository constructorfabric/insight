"""The bronze test schemas against the tables the connectors actually created.

A spec can seed only what its table's schema declares, so a schema that has drifted
from the real table silently makes a real source state unseedable — a pull request with
no merge time, or a diffstat row's collection stamp. Two such gaps had to be closed
before the pull-request metrics could be tested at all (#3362), and neither showed up
as a failure: every existing spec happened to set the columns involved.

The oracle is the warehouse the session fixture built, which `destination-clickhouse`
created from each connector's own catalogue. It used to be a committed DDL snapshot;
that snapshot no longer carries bronze, because a file it carried would have been
applied first and decided the shape before the destination ever ran.

Three rules, and deliberately not a fourth:

* a schema file naming a bronze table no connector creates is a stream that was
  renamed or dropped; the six `config.*` fixtures have no connector and are skipped;
* a column no bronze table has is always a mistake, so that is checked everywhere;
* exact column parity is a RATCHET over `PARITY_TABLES` — those hold it today and
  must keep it. Most schemas are partial by choice and bringing them in is separate
  work, so they are not listed.

Nullability is left alone on purpose. Declaring an identity column non-null is what
forces a spec to state it, and the columns widened in #3362 were widened because a
real source state needed them, which no rule derived from the table can tell.
"""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml
from insight_datapath import clickhouse as ch
from insight_datapath.instance import InstanceConfig

REPO_ROOT = Path(__file__).resolve().parents[3]
SCHEMA_DIR = REPO_ROOT / "tests/datapath/metrics/schemas"

BRONZE_DATABASE_PREFIX = "bronze_"
BRONZE_DATABASE_PATTERN = "bronze\\_%"

#: Tables whose test schema mirrors the real table column for column. A table joins
#: this list once its schema is complete; it never leaves.
PARITY_TABLES = (
    "bronze_bitbucket_cloud.pull_requests",
    "bronze_bitbucket_cloud.pull_request_diffstat",
    "bronze_gitlab.pull_requests",
)


def _declared_columns() -> dict[str, set[str]]:
    """Every table a data-path schema file declares, and the columns it allows."""
    declared: dict[str, set[str]] = {}
    for path in sorted(SCHEMA_DIR.glob("*.yaml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for table, spec in (document.get("schemas") or {}).items():
            declared[table] = set((spec.get("properties") or {}).keys())
    return declared


DECLARED = _declared_columns()


@pytest.fixture(scope="session")
def created(instance_cfg: InstanceConfig, warehouse_schema: int) -> dict[str, set[str]]:
    """Every `bronze_*` table the connectors created, and its columns."""
    tables: dict[str, set[str]] = {}
    for database, table, column in ch.query(
        instance_cfg,
        "SELECT database, table, name FROM system.columns "
        f"WHERE database LIKE '{BRONZE_DATABASE_PATTERN}'",
    ):
        tables.setdefault(f"{database}.{table}", set()).add(str(column))
    return tables


def test_the_connectors_created_bronze_tables(created: dict[str, set[str]]) -> None:
    """A warehouse this file cannot read would make every rule below vacuous."""
    assert len(created) > 100, f"only {len(created)} bronze tables were created"
    assert DECLARED, "no data-path schema files were found"


@pytest.mark.parametrize("table", sorted(DECLARED))
def test_a_schema_declares_no_column_the_table_lacks(
    table: str, created: dict[str, set[str]]
) -> None:
    """A column outside the real table cannot be seeded and is a typo or a rename."""
    if not table.startswith(BRONZE_DATABASE_PREFIX):
        pytest.skip(f"{table} is not bronze; no connector creates it")
    assert table in created, (
        f"{table} has a schema file but no connector creates it — the stream was "
        f"renamed or dropped, and every spec seeding it fails at insert time"
    )
    unknown = sorted(DECLARED[table] - created[table])
    assert not unknown, f"{table}: declared but absent from the created table: {unknown}"


@pytest.mark.parametrize("table", PARITY_TABLES)
def test_a_parity_table_declares_every_column_the_table_has(
    table: str, created: dict[str, set[str]]
) -> None:
    """The ratchet: a column added to one of these tables must reach its schema.

    Without it a spec cannot seed the new column, and nothing fails — the gap only
    surfaces when somebody tries to write the test that needs it.
    """
    assert table in created, f"{table} was not created by any connector"
    assert table in DECLARED, f"{table} has no data-path schema file"
    missing = sorted(created[table] - DECLARED[table])
    assert not missing, (
        f"{table}: created but not declared: {missing}. Add them to "
        f"tests/datapath/metrics/schemas/{table}.yaml"
    )
