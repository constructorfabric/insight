"""Typed CSV → bronze INSERT + per-test TRUNCATE ledger.

Each fixture's `bronze/<schema>.<table>.csv` is loaded into a pandas
DataFrame, then INSERTed into ClickHouse. Column types are looked up via
`system.columns` so that empty cells become SQL NULL, dates parse correctly,
and numeric columns are typed.

The seeder records every `(schema, table)` it touches in a per-test ledger
so the next test's setup can TRUNCATE exactly those tables — not DROP, not
the entire database.
"""

from __future__ import annotations

import ast
import logging
from dataclasses import dataclass, field
from typing import Iterable

import pandas as pd

from e2e_lib import clickhouse as ch
from e2e_lib.config import SessionConfig
from e2e_lib.fixture_loader import BronzeCsv, Fixture, SilverCsv

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
    """Per-session helper that loads fixture CSVs into bronze tables.

    The seeder is stateful between tests via the `ledger` attribute. The csv-rig
    integration calls `truncate_touched()` BEFORE seeding each new test, so the
    first test sees an empty ledger (no-op TRUNCATE) and subsequent tests TRUNCATE
    only what the prior test wrote.
    """

    def __init__(self, cfg: SessionConfig):
        self.cfg = cfg
        self.ledger = TouchedLedger()

    # ------------------------------------------------------------------
    # Per-test API
    # ------------------------------------------------------------------

    def truncate_touched(self) -> int:
        """TRUNCATE every (schema, table) recorded by the last test. Returns count truncated."""
        drained = list(self.ledger.drain())
        for schema, table in drained:
            self._truncate(schema, table)
        return len(drained)

    def seed(self, fixture: Fixture) -> None:
        """Load every bronze AND silver CSV in the fixture into ClickHouse."""
        for csv in fixture.bronze_csvs:
            self._seed_one(csv)
            self.ledger.record(csv.schema, csv.table)
        for csv in fixture.silver_csvs:
            self._seed_one(csv)
            self.ledger.record(csv.schema, csv.table)

    # ------------------------------------------------------------------
    # internals
    # ------------------------------------------------------------------

    def _seed_one(self, csv: BronzeCsv | SilverCsv) -> None:
        column_types = self._fetch_column_types(csv.schema, csv.table)
        if not column_types:
            raise SeederError(
                f"table {csv.full_name} not found in system.columns "
                f"(bronze: check connector airbyte placeholder; "
                f"silver: ensure migrations have run + dbt placeholders exist)"
            )

        # Truncate BEFORE insert so we don't accumulate across sessions.
        # ReplacingMergeTree doesn't dedup on read without FINAL, and several
        # downstream views (e.g. insight.ic_kpis reading silver.class_focus_metrics)
        # do NOT use FINAL — so duplicate rows would inflate sums.
        # The ledger-based truncate_touched() handles within-session isolation;
        # this handles across-session.
        self._truncate(csv.schema, csv.table)

        # Read CSV: pandas defaults turn empty cells into NaN, which we then
        # convert to None for ClickHouse NULL.
        df = pd.read_csv(csv.path, keep_default_na=True)
        df = self._coerce_types(df, column_types, source=csv.path)

        missing_cols = [c for c in df.columns if c not in column_types]
        if missing_cols:
            raise SeederError(
                f"{csv.path}: columns not in {csv.full_name}: {missing_cols} "
                f"(CH columns: {list(column_types)})"
            )

        def _to_ch(v):
            # `pd.isna` on a list raises — short-circuit container types first.
            if isinstance(v, (list, dict)):
                return v
            try:
                return None if pd.isna(v) else v
            except (TypeError, ValueError):
                return v

        rows = [tuple(_to_ch(v) for v in row) for row in df.itertuples(index=False, name=None)]
        cols = list(df.columns)
        LOG.info("seeding %s.%s: %d rows × %d cols", csv.schema, csv.table, len(rows), len(cols))
        with ch.client(self.cfg, database=csv.schema) as c:
            c.insert(table=csv.table, data=rows, column_names=cols)

    def _truncate(self, schema: str, table: str) -> None:
        LOG.debug("TRUNCATE %s.%s", schema, table)
        ch.execute(self.cfg, f"TRUNCATE TABLE IF EXISTS `{schema}`.`{table}`")

    def _fetch_column_types(self, schema: str, table: str) -> dict[str, str]:
        rows = ch.query(
            self.cfg,
            f"SELECT name, type FROM system.columns WHERE database = '{schema}' AND table = '{table}'",
        )
        return {name: ctype for name, ctype in rows}

    @staticmethod
    def _coerce_types(df: pd.DataFrame, column_types: dict[str, str], *, source) -> pd.DataFrame:
        """Best-effort pandas-side coercion to match CH column types.

        We handle the cases where pandas' read_csv default would mis-type:
          - Date / DateTime: parse strings → pandas datetime
          - Boolean: CH wants 0/1 or actual bool
          - Array(...): CSV cell looks like `['a','b']`; parse via ast.literal_eval
          - Enum8: pandas string is fine, but Nullable(Enum8) needs None handling
        Empty cells already arrive as NaN; we convert to None just before INSERT.
        """
        out = df.copy()
        for col in out.columns:
            ch_type = column_types.get(col, "")
            base = _strip_nullable(ch_type)
            if base.startswith("Date"):
                try:
                    out[col] = pd.to_datetime(out[col], errors="coerce")
                except Exception as e:
                    raise SeederError(f"{source}: failed to parse Date column {col!r}: {e}") from e
            elif base in ("Bool", "Boolean"):
                out[col] = out[col].map({"true": True, "false": False, True: True, False: False, 1: True, 0: False})
            elif base.startswith("Array("):
                # `['a','b']` (str) → ['a', 'b'] (list). Empty cells → [].
                def _parse_array(v):
                    if v is None or (isinstance(v, float) and pd.isna(v)):
                        return []
                    if isinstance(v, list):
                        return v
                    try:
                        return list(ast.literal_eval(v))
                    except (ValueError, SyntaxError) as e:
                        raise SeederError(
                            f"{source}: column {col!r} (CH type {ch_type}) "
                            f"could not parse {v!r} as a Python list: {e}"
                        ) from e

                out[col] = out[col].apply(_parse_array)
        return out


def _strip_nullable(ch_type: str) -> str:
    """`Nullable(DateTime64(6))` -> `DateTime64(6)`; `String` -> `String`."""
    if ch_type.startswith("Nullable(") and ch_type.endswith(")"):
        return ch_type[len("Nullable(") : -1]
    return ch_type
