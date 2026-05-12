#!/usr/bin/env python3
"""Compare desired connection tags to remote.

CLI:
  diff_connection_tags.py --desired-tags PATH --remote-connection PATH

Stdout: 'same' or 'differ'.
Exit:   0 same, 1 differ.

NB: this diff is NOT data-affecting (per ADR-0008) — caller MUST NOT trigger
sync on differ.
"""
import argparse, json, sys


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--desired-tags", required=True)
    p.add_argument("--remote-connection", required=True)
    args = p.parse_args()
    with open(args.desired_tags, "r", encoding="utf-8") as f:
        desired = json.load(f)
    with open(args.remote_connection, "r", encoding="utf-8") as f:
        remote = json.load(f)
    actual = remote.get("tags", [])
    out = "same" if sorted(desired) == sorted(actual) else "differ"
    sys.stdout.write(out)
    return 0 if out == "same" else 1


if __name__ == "__main__":
    sys.exit(main())
