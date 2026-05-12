#!/usr/bin/env python3
"""Select first connectionId for a given source_id.

Stdin:  JSON list of connection objects.
CLI:    select_connection_by_source.py <source_id>
Stdout: connectionId string (empty if no match).
Exit:   0 always.
"""
import json
import sys


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("select_connection_by_source: expected 1 arg (source_id)\n")
        return 2
    target = sys.argv[1]
    for c in json.load(sys.stdin):
        if c.get("sourceId") == target:
            print(c.get("connectionId", ""))
            return 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
