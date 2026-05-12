#!/usr/bin/env python3
"""Filter connections to those bound to a given source_id.

Stdin:  JSON list of connection objects.
CLI:    select_connections_by_source.py <source_id>
Stdout: One JSON object per matching connection: {connectionId, tags}.
Exit:   0 always.
"""
import json
import sys


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("select_connections_by_source: expected 1 arg (source_id)\n")
        return 2
    target = sys.argv[1]
    for c in json.load(sys.stdin):
        if c.get("sourceId") == target:
            print(
                json.dumps(
                    {
                        "connectionId": c.get("connectionId"),
                        "tags": c.get("tags", []),
                    }
                )
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
