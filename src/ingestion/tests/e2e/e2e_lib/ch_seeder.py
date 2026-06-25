"""Seed resolved YAML records into bronze + per-test TRUNCATE ledger.

Each `bronze.<db>.<table>` entry in a `*.test.yaml` is a list of records that the
fixture-loader has already `$ref`-resolved, padded to the table schema, and
validated. This module coerces each record's values to the ClickHouse column
types (looked up via `system.columns`) and INSERTs them.

The seeder records every `(schema, table)` it touches in a per-test ledger so the
next test's setup can TRUNCATE exactly those tables — not DROP, not the whole DB.
Duplicate records are inserted physically so dedup is exercised at the
bronze/silver layer (cpt-bronze-to-api-e2e-dod-yaml-bronze-seed).
"""

from __future__ import annotations

import logging
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Iterable

from e2e_lib import clickhouse as ch
from e2e_lib.config import SessionConfig

LOG = logging.getLogger("e2e.seeder")


class SeederError(RuntimeError):
    pass


@dataclass
class TouchedLedger:
    """Tables touched by the current test, for next-test TRUNCATE."""

    tables: set[tuple[str, str]] = field(default_factory=set)

    def record(self, schema: str, table: str) -> None:
        self.tables.add((schema, table))

    def drain(self) -> Iterable[tuple[str, str]]:
        out = list(self.tables)
        self.tables.clear()
        return out


class CHSeeder:
    """Per-session helper that loads resolved test records into bronze tables."""

    def __init__(self, cfg: SessionConfig):
        self.cfg = cfg
        self.ledger = TouchedLedger()

    # ------------------------------------------------------------------
    # Per-test API
    # ------------------------------------------------------------------

    def truncate_touched(self) -> int:
        """TRUNCATE every (schema, table) recorded by the last test."""
        drained = list(self.ledger.drain())
        for schema, table in drained:
            self._truncate(schema, table)
        return len(drained)

    def truncate_table(self, schema: str, table: str) -> None:
        """Public one-off TRUNCATE for tables dbt does not rebuild — e.g. the
        enrich-output staging tables, which the enrich binary appends to and dbt
        never owns, so they must be cleared between tests to avoid accumulation.
        """
        self._truncate(schema, table)

    def seed_bronze(self, bronze: dict[str, list[dict]]) -> None:
        """Seed every `<db>.<table>: [records]` entry of a TestYaml."""
        for table_fqn, rows in bronze.items():
            schema, _, table = table_fqn.partition(".")
            self.seed_records(schema, table, rows)
            self.ledger.record(schema, table)

    def seed_records(self, schema: str, table: str, rows: list[dict]) -> None:
        """TRUNCATE then INSERT `rows` (list of field maps) into `<schema>.<table>`."""
        column_types = self._fetch_column_types(schema, table)
        if not column_types:
            raise SeederError(
                f"table {schema}.{table} not found in system.columns "
                f"(bronze: check the placeholder; silver: ensure migrations + dbt ran)"
            )
        self._truncate(schema, table)
        if not rows:
            return

        cols = list(column_types)
        ch_rows = []
        for rec in rows:
            unknown = [k for k in rec if k not in column_types]
            if unknown:
                raise SeederError(
                    f"{schema}.{table}: record has columns not in the table: {unknown}"
                )
            ch_rows.append(tuple(self._coerce(rec.get(c), column_types[c], c) for c in cols))

        LOG.info("seeding %s.%s: %d rows × %d cols", schema, table, len(ch_rows), len(cols))
        with ch.client(self.cfg, database=schema) as c:
            c.insert(table=table, data=ch_rows, column_names=cols)

    # ------------------------------------------------------------------
    # internals
    # ------------------------------------------------------------------

    def _truncate(self, schema: str, table: str) -> None:
        # Views are stateless — they hold no data of their own, so there is
        # nothing to TRUNCATE (and ClickHouse rejects `TRUNCATE … View` with
        # NOT_IMPLEMENTED, code 48). Several staging models are materialized as
        # views over overwrite-mode bronze (e.g. jira__task_users,
        # jira__bronze_promoted): the bronze table underneath is itself in the
        # ledger and gets truncated, which clears the view's content. Skipping
        # the view keeps per-test isolation correct without erroring.
        engine = self._fetch_engine(schema, table)
        if engine is not None and "View" in engine:
            return
        ch.execute(self.cfg, f"TRUNCATE TABLE IF EXISTS `{schema}`.`{table}`")

    def _fetch_engine(self, schema: str, table: str) -> str | None:
        rows = ch.query(
            self.cfg,
            f"SELECT engine FROM system.tables WHERE database = '{schema}' AND name = '{table}'",
        )
        return rows[0][0] if rows else None

    def _fetch_column_types(self, schema: str, table: str) -> dict[str, str]:
        rows = ch.query(
            self.cfg,
            f"SELECT name, type FROM system.columns WHERE database = '{schema}' AND table = '{table}'",
        )
        return {name: ctype for name, ctype in rows}

    @staticmethod
    def _coerce(value: Any, ch_type: str, col: str) -> Any:
        """Coerce a YAML-parsed value to what clickhouse-connect expects for `ch_type`."""
        if value is None:
            return None
        base = _strip_nullable(ch_type)
        if base.startswith("Date"):
            if isinstance(value, (datetime,)):
                return value
            try:
                return datetime.fromisoformat(str(value))
            except ValueError as e:
                raise SeederError(f"column {col!r} ({ch_type}): bad datetime {value!r}: {e}") from e
        if base in ("Bool", "Boolean"):
            if isinstance(value, bool):
                return value
            norm = str(value).strip().lower()
            if norm in ("true", "1"):
                return True
            if norm in ("false", "0"):
                return False
            raise SeederError(
                f"column {col!r} ({ch_type}): non-boolean value {value!r} "
                f"(use true/false)"
            )
        # String / Decimal / Float / Int / Array / JSON: pass through. clickhouse-connect
        # accepts int→Decimal, str→JSON, list→Array, str→String as-is.
        return value


def _strip_nullable(ch_type: str) -> str:
    if ch_type.startswith("Nullable(") and ch_type.endswith(")"):
        return ch_type[len("Nullable(") : -1]
    return ch_type
