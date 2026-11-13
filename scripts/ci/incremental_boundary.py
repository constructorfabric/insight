#!/usr/bin/env python3
"""Incremental boundary guard — no dbt model may take a table-wide watermark.

A class table is written by several producers, each stamping `_version` on its
own clock, and a staging table by several instances of one connector, each
syncing on its own schedule. A boundary taken as one `max(...)` over the whole
of `{{ this }}` lets one of them raise it past another's rows, which are then
never read again and never reported (#2927, #752). Silver models use
`silver_incremental_watermark(keys)`; any other model compares against
`instance_incremental_watermark(column)`, which looks up the row's own instance.

Checks
  1. table-wide — no model under src/ingestion may close a scalar subquery
                  `(SELECT max(...) FROM {{ this }})`. A `max` grouped by
                  instance is allowed. Comments are ignored.

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
# `max(` and a `FROM {{ this }}` whose subquery closes without a GROUP BY or another SELECT.
TABLE_WIDE = re.compile(
    r"\bmax\((?:(?!\bSELECT\b)[\s\S])*?\bFROM\s+\{\{\s*this\s*\}\}"
    r"(?:(?!\bGROUP\s+BY\b|\bSELECT\b)[^()])*\)",
    re.IGNORECASE,
)


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
        return [Finding(path, "table-wide `max(...) FROM {{ this }}` boundary; use an instance-scoped watermark macro")]
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
