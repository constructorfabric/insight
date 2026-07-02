#!/usr/bin/env python3
"""connector_silver_coverage.py — does every ingesting connector actually reach
a dashboard-readable silver class?

A connector can be fully configured and syncing to bronze, yet contribute NOTHING
to any dashboard if its data is never promoted into the `silver.class_*` tables the
gold serving views read. That is a silent, build-time gap: nothing fails, the
dashboards just render "No data".

Concrete instance this guards against (TAF-199): the YouTrack connector ships only
`youtrack__bronze_promoted` and is tagged into no `silver:class_*` union, so on a
YouTrack tenant the Task Delivery dashboards are permanently empty. Likewise there
is no GitLab connector at all.

The invariant
-------------
Every connector that promotes raw data to bronze (`*__bronze_promoted.sql`) must
have at least one dbt model tagged into a silver class union (`silver:class_*`).
A connector that reaches bronze but no silver class is "stranded": it ingests data
that no dashboard can ever read.

This is a pure static check over the dbt model tags — no warehouse, no network.

Usage:
  connector_silver_coverage.py            # report, exit 0
  connector_silver_coverage.py --check    # exit 1 if any connector is stranded
  connector_silver_coverage.py --check --waive task-tracking/youtrack,crm/salesforce
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# repo-root/src/ingestion/connectors/<category>/<name>/dbt/*.sql
CONNECTORS = Path(__file__).resolve().parents[2] / "src" / "ingestion" / "connectors"

_TAGS_RE = re.compile(r"tags\s*=\s*\[([^\]]*)\]", re.DOTALL)
_STR_RE = re.compile(r"""['"]([^'"]+)['"]""")


def model_tags(sql: str) -> set[str]:
    """All tag strings declared in a dbt model's `config(tags=[...])`."""
    tags: set[str] = set()
    for block in _TAGS_RE.findall(sql):
        tags.update(_STR_RE.findall(block))
    return tags


def scan() -> list[tuple[str, bool, bool, set[str]]]:
    """One row per connector: (name, reaches_bronze, reaches_silver_class, silver_tags)."""
    rows = []
    for dbt_dir in sorted(CONNECTORS.glob("*/*/dbt")):
        name = f"{dbt_dir.parent.parent.name}/{dbt_dir.parent.name}"
        reaches_bronze = False
        silver_class_tags: set[str] = set()
        for sql_file in dbt_dir.glob("*.sql"):
            if sql_file.name.endswith("__bronze_promoted.sql"):
                reaches_bronze = True
            for tag in model_tags(sql_file.read_text(encoding="utf-8", errors="replace")):
                if tag.startswith("silver:class_"):
                    silver_class_tags.add(tag)
                if tag.startswith("bronze"):
                    reaches_bronze = True
        rows.append((name, reaches_bronze, bool(silver_class_tags), silver_class_tags))
    return rows


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="exit 1 if any connector is stranded")
    ap.add_argument("--waive", default="", help="comma list of connectors allowed to be bronze-only")
    args = ap.parse_args()
    waived = {w.strip() for w in args.waive.split(",") if w.strip()}

    if not CONNECTORS.is_dir():
        print(f"FATAL: connectors dir not found: {CONNECTORS}")
        sys.exit(2)

    stranded = []
    print("== connector → silver-class coverage ==")
    for name, bronze, silver, tags in scan():
        if bronze and not silver:
            tag = "WAIVED " if name in waived else "STRANDED"
            print(f"  {tag} {name}: reaches bronze, no silver:class_* union")
            if name not in waived:
                stranded.append(name)
        elif silver:
            print(f"  ok       {name}: {', '.join(sorted(tags))}")
        else:
            print(f"  --       {name}: no bronze_promoted (not an ingest source yet)")

    print(f"\nsummary: stranded={len(stranded)}")
    if args.check and stranded:
        print(f"\nFAILED: connectors reach bronze but no dashboard-readable silver class: {stranded}")
        print("Fix: tag a model into the relevant silver:class_* union, or --waive if intentional.")
        sys.exit(1)
    if args.check:
        print("\nPASSED")


if __name__ == "__main__":
    main()
