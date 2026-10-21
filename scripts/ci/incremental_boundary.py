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

A second check used to read the watermark keys back against the committed DDL
snapshot, so a misspelt key failed here rather than at the first run. That
snapshot is gone (#3562) and nothing else states a class relation's columns
ahead of time — a class is a positional union of whatever staging arms exist,
so the column set is only known once dbt has built it. Rather than leave a
check that warns on every relation and exits 0, it is removed: a misspelt key
now fails the run that builds the model, with UNKNOWN_IDENTIFIER naming it.

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
SKIPPED_DIRS = ("dbt/macros", "dbt/tests", "dbt/target", "scripts")

LINE_COMMENT = re.compile(r"--[^\n]*")
JINJA_COMMENT = re.compile(r"\{#.*?#\}", re.DOTALL)
TABLE_WIDE = re.compile(r"max\(\s*_version\s*\)\s+FROM\s+\{\{\s*this\s*\}\}", re.IGNORECASE)


@dataclass(frozen=True)
class Finding:
    path: str
    message: str


def strip_comments(sql: str) -> str:
    return LINE_COMMENT.sub("", JINJA_COMMENT.sub("", sql))


def has_table_wide_boundary(sql: str) -> bool:
    return TABLE_WIDE.search(strip_comments(sql)) is not None


def check_model(path: str, sql: str) -> list[Finding]:
    if has_table_wide_boundary(sql):
        return [Finding(path, "table-wide `max(_version) FROM {{ this }}` boundary; use silver_incremental_watermark")]
    return []


def models(root: Path) -> list[Path]:
    base = root / INGESTION
    return sorted(
        p for p in base.rglob("*.sql") if not any(part in p.relative_to(base).as_posix() for part in SKIPPED_DIRS)
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, default=Path())
    args = parser.parse_args()

    errors: list[Finding] = []
    for path in models(args.root):
        errors += check_model(path.relative_to(args.root).as_posix(), path.read_text())

    for f in errors:
        print(f"error: {f.path}: {f.message}", file=sys.stderr)
    print(f"incremental boundary: {len(errors)} error(s) over {len(models(args.root))} model(s)")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
