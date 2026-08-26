#!/usr/bin/env python3
"""Find connections tagged `insight` whose connector is no longer known.

CLI: find_orphan_connections.py <known_names_path> <sources_path> <connections_path>
Stdout: TSV `connection_id\tsource_id\tconnector_slug` per orphan.
Exit:   0 always; 2 on bad arg count.

The three inputs are read from files, not passed inline: Linux caps a
single argv string at MAX_ARG_STRLEN (128 KiB), and the workspace-wide
`connections/list` payload carries a full syncCatalog per connection, so
an inline blob makes execve fail with E2BIG once enough streams exist.
Callers hand over paths (a temp file or a `<(...)` process substitution).

Source names follow `{connector}-{source-id}-{tenant}`. Connector slugs
themselves can contain dashes (e.g. `ms-entra`, `bitbucket-cloud`,
`github-directory`), so a naive `split("-")[0]` would mis-identify
`ms-entra-main-default` as connector `ms` and incorrectly cascade-delete
a healthy connection. We resolve the connector by **longest-prefix
match** against the `known` set: the connector slug is the longest
known name for which `<slug>-` is a prefix of the source name. Only
when nothing matches do we treat the connection as a real orphan.
"""

import json
import sys
from pathlib import Path
from typing import Any


def _load_json(path: str) -> Any:
    with Path(path).open(encoding="utf-8") as f:
        return json.load(f)


def _resolve_connector(source_name: str, known: set[str]) -> str | None:
    """Longest known slug for which `<slug>-` is a prefix of source_name."""
    candidates = [k for k in known if source_name.startswith(f"{k}-")]
    return max(candidates, key=len) if candidates else None


def main() -> int:
    if len(sys.argv) != 4:
        sys.stderr.write("find_orphan_connections: expected 3 paths (known, sources, connections)\n")
        return 2
    known: set[str] = set(_load_json(sys.argv[1]))
    sources: dict[str, Any] = {s["sourceId"]: s for s in _load_json(sys.argv[2])}
    connections = _load_json(sys.argv[3])
    for c in connections:
        tags = c.get("tags", []) or []
        tag_names = [t.get("name") if isinstance(t, dict) else t for t in tags]
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
