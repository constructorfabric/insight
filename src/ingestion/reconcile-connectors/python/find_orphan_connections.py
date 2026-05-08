#!/usr/bin/env python3
"""Find connections tagged `insight` whose connector is no longer known.

CLI: find_orphan_connections.py <known_names_json> <sources_json> <connections_json>
Stdout: TSV `connection_id\tsource_id\tconnector_slug` per orphan.
Exit:   0 always; 2 on bad arg count.

Source names follow `{connector}-{source-id}-{tenant}`. Connector slugs
themselves can contain dashes (e.g. `ms-entra`, `bitbucket-cloud`,
`github-v2`), so a naive `split("-")[0]` would mis-identify
`ms-entra-main-default` as connector `ms` and incorrectly cascade-delete
a healthy connection. We resolve the connector by **longest-prefix
match** against the `known` set: the connector slug is the longest
known name for which `<slug>-` is a prefix of the source name. Only
when nothing matches do we treat the connection as a real orphan.
"""
import json
import sys
from typing import Any, Dict, Optional, Set


def _resolve_connector(source_name: str, known: Set[str]) -> Optional[str]:
    """Longest known slug for which `<slug>-` is a prefix of source_name."""
    candidates = [k for k in known if source_name.startswith(f"{k}-")]
    return max(candidates, key=len) if candidates else None


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
        source_name = src.get("name") or ""
        slug = _resolve_connector(source_name, known)
        if slug is None:
            cid = c.get("connectionId")
            sid = src.get("sourceId")
            # `<unknown>` in the third column makes the diagnostic
            # explicit; the previous `split("-")[0]` value falsely
            # implied the connector slug had been parsed correctly.
            print("\t".join([cid or "", sid or "", "<unknown>"]))
    return 0


if __name__ == "__main__":
    sys.exit(main())
