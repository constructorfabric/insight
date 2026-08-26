#!/usr/bin/env python3
"""Read one sweep plan into the fields the shell records with.

    sweep_read_plan.py  < plan.json

Writes a single unit-separated line: rows, unmappable count, undatable count,
seal, row count. One process and one parse, so the shell neither starts Python
five times for the same document nor carries five unguarded command
substitutions — under the caller's `set -e`, any one of those could abort the
reconcile tick this sweep is forbidden to disturb.
"""

from __future__ import annotations

import json
import sys

SEPARATOR = "\037"


def fields(plan: dict) -> list[str]:
    rows = plan.get("rows") or []
    seal = plan.get("seal")
    return [
        json.dumps(rows),
        str(len(plan.get("unmappable_jobs") or [])),
        str(len(plan.get("undatable_jobs") or [])),
        json.dumps([seal] if seal else []),
        str(len(rows)),
    ]


def main() -> int:
    sys.stdout.write(SEPARATOR.join(fields(json.load(sys.stdin))))
    return 0


if __name__ == "__main__":
    sys.exit(main())
