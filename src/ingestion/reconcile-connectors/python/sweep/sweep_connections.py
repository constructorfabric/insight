#!/usr/bin/env python3
"""Join managed connectors to their mover connections (connector-health §3.2).

Pure over values: takes the mover's connection listing on stdin and the
tick's `<connector>\\t<connection name>` pairs as an argument, prints the
connection-id to connector map plus the configured set.

The pairs come from the same function that named the connections, so nothing
here parses a connection name — a connector name may contain hyphens, which
makes splitting the name ambiguous.

    sweep_connections.py "<connector>\\t<connection name>\\n..." < connections.json
"""

from __future__ import annotations

import json
import sys
from typing import Any


def join(connections: list[dict[str, Any]], pairs: str) -> dict[str, Any]:
    by_name = {c.get("name"): c.get("connectionId") for c in connections}

    mapping: dict[str, str] = {}
    configured: list[str] = []
    for line in pairs.splitlines():
        if not line.strip():
            continue
        connector, connection_name = line.split("\t", 1)
        # Configured means the tick manages it, whether or not the mover has
        # caught up: a connector configured a minute ago has no connection yet,
        # and must still read as configured rather than never configured.
        configured.append(connector)
        connection_id = by_name.get(connection_name)
        if connection_id:
            mapping[connection_id] = connector

    return {"connections": mapping, "configured": sorted(configured)}


def main() -> int:
    pairs = sys.argv[1] if len(sys.argv) > 1 else ""
    json.dump(join(json.load(sys.stdin), pairs), sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
