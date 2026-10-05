#!/usr/bin/env python3
"""Incremental boundary guard — no dbt model may take a table-wide watermark.

A class table is written by several producers, each stamping `_version` on its
own clock. `WHERE _version > (SELECT max(_version) FROM {{ this }})` lets one
producer's newest row raise the boundary past another producer's rows, which
are then never read again and never reported (#2927). Every incremental model
uses `silver_incremental_watermark(keys)` instead.

Checks
  1. table-wide — no model under src/ingestion may compare against
                  `max(_version) FROM {{ this }}`. Comments are ignored.
  2. keys       — every key passed to `silver_incremental_watermark` must be a
                  column of the model's relation in the connectors-ddl snapshot,
                  so a misspelt key fails here rather than at the first run.
                  A relation the snapshot does not hold is a warning.

Usage: python3 scripts/ci/incremental_boundary.py [--root DIR]
Exit:  0 clean (warnings allowed), 1 on any error.
"""

# ruff: noqa: T201  — stdout/stderr IS this script's CI report (cf. changed.py).

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

INGESTION = Path("src/ingestion")
DDL_DIR = INGESTION / "scripts" / "connectors-ddl"
SKIPPED_DIRS = ("dbt/macros", "dbt/tests", "dbt/target", "scripts")

LINE_COMMENT = re.compile(r"--[^\n]*")
JINJA_COMMENT = re.compile(r"\{#.*?#\}", re.DOTALL)
TABLE_WIDE = re.compile(r"max\(\s*(?:`?\w+`?\.)?`?_version`?\s*\)\s+FROM\s+\{\{\s*this\s*\}\}", re.IGNORECASE)
WATERMARK_KEYS = re.compile(r"silver_incremental_watermark\(\s*\[([^\]]*)\]")
QUOTED = re.compile(r"'([^']+)'|\"([^\"]+)\"")
SCHEMA = re.compile(r"\bschema\s*=\s*'([^']+)'")
CREATE_TABLE = re.compile(r"CREATE TABLE (?:IF NOT EXISTS )?`?(\w+)`?\.`?(\w+)`?\s*\((.*?)\n\)\s*ENGINE", re.DOTALL)
DDL_COLUMN = re.compile(r"^\s*`?(\w+)`?\s+\S", re.MULTILINE)


@dataclass(frozen=True)
class Finding:
    path: str
    message: str


def strip_comments(sql: str) -> str:
    return LINE_COMMENT.sub("", JINJA_COMMENT.sub("", sql))


def has_table_wide_boundary(sql: str) -> bool:
    return TABLE_WIDE.search(strip_comments(sql)) is not None


def watermark_keys(sql: str) -> list[list[str]]:
    calls = WATERMARK_KEYS.findall(strip_comments(sql))
    return [[a or b for a, b in QUOTED.findall(keys)] for keys in calls]


def relation_of(path: Path, sql: str) -> tuple[str, str]:
    """The relation a model writes: its configured schema, or silver, and its file name."""
    match = SCHEMA.search(sql)
    return (match.group(1) if match else "silver", path.stem)


def ddl_columns(ddl: str) -> dict[tuple[str, str], set[str]]:
    return {(schema, table): set(DDL_COLUMN.findall(body)) for schema, table, body in CREATE_TABLE.findall(ddl)}


def check_model(
    path: str, relation: tuple[str, str], sql: str, columns: dict[tuple[str, str], set[str]]
) -> tuple[list[Finding], list[Finding]]:
    errors: list[Finding] = []
    warnings: list[Finding] = []

    if has_table_wide_boundary(sql):
        errors.append(
            Finding(path, "table-wide `max(_version) FROM {{ this }}` boundary; use silver_incremental_watermark")
        )

    for keys in watermark_keys(sql):
        known = columns.get(relation)
        if known is None:
            warnings.append(
                Finding(path, f"{relation[0]}.{relation[1]} is not in the connectors-ddl snapshot; keys unchecked")
            )
            continue
        missing = [k for k in keys if k not in known]
        if missing:
            errors.append(
                Finding(path, f"watermark keys not columns of {relation[0]}.{relation[1]}: {', '.join(missing)}")
            )

    return errors, warnings


def models(root: Path) -> list[Path]:
    base = root / INGESTION
    return sorted(
        p for p in base.rglob("*.sql") if not any(part in p.relative_to(base).as_posix() for part in SKIPPED_DIRS)
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=Path())
    args = parser.parse_args()

    ddl = "\n".join(p.read_text() for p in sorted((args.root / DDL_DIR).glob("*.sql")))
    columns = ddl_columns(ddl)

    errors: list[Finding] = []
    warnings: list[Finding] = []
    for path in models(args.root):
        sql = path.read_text()
        e, w = check_model(path.relative_to(args.root).as_posix(), relation_of(path, sql), sql, columns)
        errors += e
        warnings += w

    for f in warnings:
        print(f"warning: {f.path}: {f.message}", file=sys.stderr)
    for f in errors:
        print(f"error: {f.path}: {f.message}", file=sys.stderr)
    print(f"incremental boundary: {len(errors)} error(s), {len(warnings)} warning(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
