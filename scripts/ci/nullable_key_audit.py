#!/usr/bin/env python3
"""nullable_key_audit.py — no dbt model may dedup on a NULLABLE key.

ReplacingMergeTree dedups by its sorting key (`order_by`). A NULL in that key
does not compare equal to itself, so rows with a NULL key never collapse — they
accumulate as silent duplicates and the metric drifts as background merges run
(the "same period, different number" class, #1330). `allow_nullable_key: 1` is
set broadly (it's needed just to *create* a table whose ORDER BY has any
nullable column), so ClickHouse won't stop you: this gate does.

For every model, the projection that defines each `order_by` column must be
provably non-null — `MD5(...)`, `CAST(... AS String)` (non-nullable), a
`coalesce(...)`/`ifNull(...)` guard, `assumeNotNull(...)`, or a passthrough of
an upstream model's already-checked key. A projection that is `Nullable(...)` or
`CAST(NULL AS ...)` is a hard failure.

This is a STATIC check (reads the .sql, no warehouse/dbt needed). Heuristic by
design — it errs toward flagging, so a flagged model is reviewed, not silently
shipped.

Usage:
  nullable_key_audit.py            # report, exit 0
  nullable_key_audit.py --check    # exit 1 if any model dedups on a nullable key
"""
from __future__ import annotations

import argparse
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
INGEST = ROOT / "src" / "ingestion"

ORDER_BY_RE = re.compile(r"order_by\s*=\s*\[([^\]]*)\]")
NULLABLE_RE = re.compile(r"Nullable\s*\(", re.IGNORECASE)
CAST_NULL_RE = re.compile(r"CAST\s*\(\s*NULL\b", re.IGNORECASE)
def projections_for(text: str, col: str) -> list[str]:
    """Return every SELECT expression projected `AS <col>`, isolated from its
    sibling columns by a paren-depth-aware backward scan.

    Walking left from `AS <col>`, commas *inside* parentheses (e.g. the args of
    `MD5(concat(a, b, c))`) sit at depth > 0 and are skipped; the expression
    starts at the first comma seen at depth 0 (the boundary with the previous
    column). This is what makes `CAST('' AS String) AS unique_key` read as just
    that cast and not pick up a neighbour's `CAST(NULL …)`.
    """
    out: list[str] = []
    for m in re.finditer(rf"\bAS\s+{re.escape(col)}\b", text, re.IGNORECASE):
        i = m.start() - 1
        depth = 0
        while i >= 0:
            c = text[i]
            if c == ")":
                depth += 1
            elif c == "(":
                depth -= 1
            elif c == "," and depth == 0:
                break
            i -= 1
        out.append(text[i + 1 : m.start()])
    return out


def audit_model(path: pathlib.Path) -> list[tuple[str, str, str]]:
    """Return (severity, col, note) findings for one model.

    Flags only the unambiguous, breaking anti-pattern: a dedup-key column whose
    own projection is `Nullable(...)` or `CAST(NULL ...)`. Everything else (MD5,
    CAST AS String, coalesce guards, upstream-key passthrough) is fine.
    """
    text = path.read_text(encoding="utf-8", errors="replace")
    m = ORDER_BY_RE.search(text)
    if not m:
        return []  # not an ordered/RMT model
    cols = [c.strip().strip("'\"") for c in m.group(1).split(",") if c.strip()]
    out: list[tuple[str, str, str]] = []
    for col in cols:
        if col.startswith("_"):
            continue  # housekeeping cols (e.g. _tracked_at) aren't the identity key
        for proj in projections_for(text, col):
            if NULLABLE_RE.search(proj) or CAST_NULL_RE.search(proj):
                out.append(
                    ("ERROR", col, "dedup key projection is Nullable / CAST(NULL …)")
                )
                break
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="exit 1 on a nullable dedup key")
    args = ap.parse_args()

    models = sorted(INGEST.rglob("*.sql"))
    ordered = 0
    errors: list[tuple[pathlib.Path, str, str]] = []
    for path in models:
        if not ORDER_BY_RE.search(path.read_text(encoding="utf-8", errors="replace")):
            continue
        ordered += 1
        for _sev, col, note in audit_model(path):
            rel = path.relative_to(ROOT)
            errors.append((rel, col, note))
            print(f"  ERROR {rel} [{col}]: {note}")

    print(
        f"\nsummary: {ordered} ordered/RMT models scanned · "
        f"{len(errors)} dedup on a NULLABLE key"
    )
    if args.check and errors:
        print(f"\nFAILED: {len(errors)} model(s) dedup on a nullable key")
        sys.exit(1)
    if args.check:
        print("\nPASSED")


if __name__ == "__main__":
    main()
