#!/usr/bin/env python3
"""Compare desired cfg_hash to remote source's stored cfg_hash annotation.

CLI:
  diff_source_config.py --desired-cfg-hash HEX --remote-source PATH

Stdout: 'same' or 'differ'.
Exit:   0 same, 1 differ.
"""
import argparse, json, sys


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--desired-cfg-hash", required=True)
    p.add_argument("--remote-source", required=True)
    args = p.parse_args()
    with open(args.remote_source, "r", encoding="utf-8") as f:
        remote = json.load(f)
    actual = remote.get("annotations", {}).get("cfg_hash", "")
    out = "same" if args.desired_cfg_hash == actual else "differ"
    sys.stdout.write(out)
    return 0 if out == "same" else 1


if __name__ == "__main__":
    sys.exit(main())
