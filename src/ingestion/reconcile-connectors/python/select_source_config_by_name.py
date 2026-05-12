#!/usr/bin/env python3
"""Select connectionConfiguration for the source matching a given name.

Stdin:  JSON list of source objects.
CLI:    select_source_config_by_name.py <name>
Stdout: JSON-encoded connectionConfiguration object (or '{}' if not found).
Exit:   0 always.
"""
import json
import sys


def main() -> int:
    if len(sys.argv) != 2:
        sys.stderr.write("select_source_config_by_name: expected 1 arg (name)\n")
        return 2
    target = sys.argv[1]
    for s in json.load(sys.stdin):
        if s.get("name") == target:
            print(json.dumps(s.get("connectionConfiguration") or {}))
            return 0
    return 0


if __name__ == "__main__":
    sys.exit(main())
