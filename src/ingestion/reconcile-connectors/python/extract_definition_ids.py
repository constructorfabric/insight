#!/usr/bin/env python3
"""Extract source_definition IDs whose `name` matches a target.

Stdin:  JSON list of source-definition objects.
CLI:    extract_definition_ids.py <name>
Stdout: JSON array of `sourceDefinitionId` strings (preserves order).
Exit:   0 always.
"""
import json
import sys
from typing import List


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("extract_definition_ids: expected 1 arg (name)\n")
        return 2
    target = sys.argv[1]
    ids: List[str] = [
        d.get("sourceDefinitionId")
        for d in json.load(sys.stdin)
        if d.get("name") == target
        and d.get("sourceDefinitionId")
        and d.get("custom") is True   # Insight namespace separation per ADR-0009
    ]
    print(json.dumps(ids))
    return 0


if __name__ == "__main__":
    sys.exit(main())
