#!/usr/bin/env python3
"""Emit connections whose source.sourceDefinitionId is in a given def-id set.

CLI:    match_connections_to_definitions.py <sources_json> <connections_json> <def_ids_json>
Stdout: One JSON object per matching connection: {connectionId, tags}.
Exit:   0 always.
"""
import json
import sys
from typing import Set


def main() -> int:
    if len(sys.argv) != 4:
        sys.stderr.write(
            "match_connections_to_definitions: expected 3 args (sources, connections, def_ids)\n"
        )
        return 2
    sources = json.loads(sys.argv[1])
    connections = json.loads(sys.argv[2])
    def_ids: Set[str] = set(json.loads(sys.argv[3]))
    matched_source_ids = {
        s["sourceId"] for s in sources if s.get("sourceDefinitionId") in def_ids
    }
    for c in connections:
        if c.get("sourceId") in matched_source_ids:
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
