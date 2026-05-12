#!/usr/bin/env python3
"""Compute lists of orphan sources/connections to delete.

CLI:
  gc_orphans.py --desired-secrets NAMES_FILE --remote-sources PATH --remote-connections PATH

NAMES_FILE: newline-separated list of expected K8s Secret names (one per
desired connector instance).

Stdout: JSON {"orphan_sources": [id,...], "orphan_connections": [id,...]}.
Exit:   0 always.
"""
import argparse, json, sys


def _load_names(path: str):
    with open(path, "r", encoding="utf-8") as f:
        return {line.strip() for line in f if line.strip()}


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--desired-secrets", required=True)
    p.add_argument("--remote-sources", required=True)
    p.add_argument("--remote-connections", required=True)
    args = p.parse_args()
    desired = _load_names(args.desired_secrets)
    with open(args.remote_sources, "r", encoding="utf-8") as f:
        sources = json.load(f)
    with open(args.remote_connections, "r", encoding="utf-8") as f:
        conns = json.load(f)
    orphan_sources = [s["sourceId"] for s in sources if s.get("name") not in desired]
    desired_source_ids = {s["sourceId"] for s in sources if s.get("name") in desired}
    orphan_connections = [c["connectionId"] for c in conns if c.get("sourceId") not in desired_source_ids]
    json.dump({"orphan_sources": orphan_sources, "orphan_connections": orphan_connections}, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
