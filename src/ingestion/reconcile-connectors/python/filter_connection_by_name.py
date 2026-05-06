#!/usr/bin/env python3
"""Filter a JSON array of Airbyte connections by name and print matching connectionIds.

CLI:
  filter_connection_by_name.py --name NAME

Stdin:  JSON array of connection objects (from ab_list_connections).
Stdout: one connectionId per line.
Exit:   0 always (caller checks line count).
"""
import argparse, json, sys

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--name", required=True)
    args = p.parse_args()
    connections = json.load(sys.stdin)
    for c in connections:
        if c.get("name") == args.name:
            cid = c.get("connectionId")
            if cid:
                print(cid)
    return 0

if __name__ == "__main__":
    sys.exit(main())
