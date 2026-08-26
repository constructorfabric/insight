#!/usr/bin/env python3
"""Render planned ledger rows as one INSERT (connector-health spec §3.2).

Pure over values, and the only place that writes ledger SQL from the sweep side,
so the escaping rule lives in exactly one function.

    sweep_insert.py <table> < rows.json

Reads the planner's `rows` array on stdin. Prints the statement, or nothing when
there is nothing to write — a tick with no new rows must not send empty SQL.
"""

from __future__ import annotations

import json
import sys
from typing import Any

COLUMNS = (
    "run_id",
    "job_id",
    "connector",
    "event",
    "status",
    "origin",
    "claim",
    "started_at",
    "duration_ms",
    "records_moved",
)


def sql_literal(value: object) -> str:
    """Single-quoted literal. Row values are data from the mover and the ledger."""
    escaped = str(value).replace("\\", "\\\\").replace("'", "\\'")
    return f"'{escaped}'"


def row_tuple(row: dict[str, Any]) -> str:
    fields = [
        sql_literal(row["run_id"]),
        sql_literal(row["job_id"]),
        sql_literal(row["connector"]),
        sql_literal(row["event"]),
        sql_literal(row["status"]),
        sql_literal(row["origin"]),
        sql_literal(row["claim"]),
        f"toDateTime64({int(row['started_at_epoch'])}, 3)",
        "NULL" if row["duration_ms"] is None else str(int(row["duration_ms"])),
        str(int(row["records_moved"])),
    ]
    return "(" + ", ".join(fields) + ")"


def statement(table: str, rows: list[dict[str, Any]]) -> str:
    if not rows:
        return ""
    tuples = ", ".join(row_tuple(row) for row in rows)
    return f"INSERT INTO {table} ({', '.join(COLUMNS)}) VALUES {tuples}"


def main() -> int:
    table = sys.argv[1]
    rows = json.load(sys.stdin)
    rendered = statement(table, rows)
    if rendered:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main())
