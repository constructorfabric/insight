#!/usr/bin/env python3
"""Find connections tagged `insight` whose connector is no longer known.

CLI: find_orphan_connections.py <known_names_json> <sources_json> <connections_json>
Stdout: TSV `connection_id\tsource_id\tconn_name` per orphan.
Exit:   0 always; 2 on bad arg count.

The connector name is encoded as the leading dash-separated segment of the
source name (matches the `{connector}-{source-id}-{tenant}` pattern).
"""
import json
import sys
from typing import Any, Dict, Set


def main() -> int:
    if len(sys.argv) != 4:
        sys.stderr.write(
            "find_orphan_connections: expected 3 args (known, sources, connections)\n"
        )
        return 2
    known: Set[str] = set(json.loads(sys.argv[1]))
    sources: Dict[str, Any] = {
        s["sourceId"]: s for s in json.loads(sys.argv[2])
    }
    connections = json.loads(sys.argv[3])
    for c in connections:
        tags = c.get("tags", []) or []
        tag_names = [
            t.get("name") if isinstance(t, dict) else t for t in tags
        ]
        if "insight" not in tag_names:
            continue
        src = sources.get(c.get("sourceId"))
        if not src:
            continue
        conn_name = (src.get("name") or "").split("-")[0]
        if conn_name and conn_name not in known:
            cid = c.get("connectionId")
            sid = src.get("sourceId")
            print("\t".join([cid or "", sid or "", conn_name]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
