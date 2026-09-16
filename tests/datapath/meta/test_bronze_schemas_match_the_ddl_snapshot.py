"""The bronze test schemas against the DDL snapshot they are declared to come from.

A spec can seed only what its table's schema declares, so a schema that has drifted
from the snapshot silently makes a real source state unseedable — a pull request with
no merge time, or a diffstat row's collection stamp. Two such gaps had to be closed
before the pull-request metrics could be tested at all (#3362), and neither showed up
as a failure: every existing spec happened to set the columns involved.

Two rules, and deliberately not a third:

* a column no snapshot table has is always a mistake, so that is checked everywhere;
* exact column parity is a RATCHET over `PARITY_TABLES` — those hold it today and
  must keep it. Most schemas are partial by choice and bringing them in is separate
  work, so they are not listed.

Nullability is left alone on purpose. Declaring an identity column non-null is what
forces a spec to state it, and the columns widened in #3362 were widened because a
real source state needed them, which no rule derived from the snapshot can tell.
"""

from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

REPO_ROOT = Path(__file__).resolve().parents[3]
SCHEMA_DIR = REPO_ROOT / "tests/datapath/metrics/schemas"
DDL_DIR = REPO_ROOT / "src/ingestion/scripts/connectors-ddl"

#: Tables whose test schema mirrors the snapshot column for column. A table joins
#: this list once its schema is complete; it never leaves.
PARITY_TABLES = (
    "bronze_bitbucket_cloud.pull_requests",
    "bronze_bitbucket_cloud.pull_request_diffstat",
    "bronze_gitlab.pull_requests",
)

_CREATE = re.compile(
    r"CREATE TABLE IF NOT EXISTS\s+(?P<name>[\w.]+)\s*\((?P<body>.*?)\)\s*ENGINE",
    re.DOTALL,
)
_COLUMN = re.compile(r"^\s*`(?P<col>[^`]+)`\s+(?P<type>.+?),?\s*$")


def _snapshot_columns() -> dict[str, set[str]]:
    """Every `bronze_*` table the generated snapshot declares, and its columns."""
    tables: dict[str, set[str]] = {}
    for sql in sorted(DDL_DIR.glob("*.sql")):
        for create in _CREATE.finditer(sql.read_text(encoding="utf-8")):
            columns = {
                match.group("col")
                for match in (_COLUMN.match(line) for line in create.group("body").splitlines())
                if match
            }
            tables[create.group("name")] = columns
    return tables


def _declared_columns() -> dict[str, set[str]]:
    """Every table a data-path schema file declares, and the columns it allows."""
    declared: dict[str, set[str]] = {}
    for path in sorted(SCHEMA_DIR.glob("*.yaml")):
        document = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
        for table, spec in (document.get("schemas") or {}).items():
            declared[table] = set((spec.get("properties") or {}).keys())
    return declared


SNAPSHOT = _snapshot_columns()
DECLARED = _declared_columns()


def test_the_snapshot_parses() -> None:
    """A snapshot this file cannot read would make every rule below vacuous."""
    assert len(SNAPSHOT) > 100, f"only {len(SNAPSHOT)} tables parsed out of the snapshot"
    assert DECLARED, "no data-path schema files were found"


@pytest.mark.parametrize("table", sorted(DECLARED))
def test_a_schema_declares_no_column_the_snapshot_lacks(table: str) -> None:
    """A column outside the real table cannot be seeded and is a typo or a rename."""
    if table not in SNAPSHOT:
        pytest.skip(f"{table} is not in the connectors DDL snapshot")
    unknown = sorted(DECLARED[table] - SNAPSHOT[table])
    assert not unknown, f"{table}: declared but absent from the snapshot: {unknown}"


@pytest.mark.parametrize("table", PARITY_TABLES)
def test_a_parity_table_declares_every_column_the_snapshot_has(table: str) -> None:
    """The ratchet: a column added to one of these tables must reach its schema.

    Without it a spec cannot seed the new column, and nothing fails — the gap only
    surfaces when somebody tries to write the test that needs it.
    """
    assert table in SNAPSHOT, f"{table} is not in the connectors DDL snapshot"
    assert table in DECLARED, f"{table} has no data-path schema file"
    missing = sorted(SNAPSHOT[table] - DECLARED[table])
    assert not missing, (
        f"{table}: in the snapshot but not declared: {missing}. Add them to "
        f"tests/datapath/metrics/schemas/{table}.yaml"
    )
