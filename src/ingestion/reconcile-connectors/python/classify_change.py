#!/usr/bin/env python3
"""Classify a source-config change as 'breaking' or 'non-breaking'.

CLI: classify_change.py <current_json> <target_json>
Stdout: 'breaking' or 'non-breaking'.

Breaking fields are those that re-tenant the source (host, hostname,
database, schema, catalog, account, workspace, tenant, orgId,
organization, repository, stream). Credential rotations / interval
tweaks are non-breaking.
"""
import json
import re
import sys


_BREAKING_RE = re.compile(
    r"^(host|hostname|database|schema|catalog|account|workspace|tenant|orgId|organization|repository|stream)$",
    re.IGNORECASE,
)


def main() -> int:
    if len(sys.argv) != 3:
        sys.stderr.write("classify_change: expected 2 args (current, target)\n")
        return 2
    current = json.loads(sys.argv[1] or "{}")
    target = json.loads(sys.argv[2] or "{}")
    keys = set(current) | set(target)
    breaking = False
    for k in keys:
        if current.get(k) != target.get(k) and _BREAKING_RE.match(k):
            breaking = True
            break
    print("breaking" if breaking else "non-breaking")
    return 0


if __name__ == "__main__":
    sys.exit(main())
