#!/usr/bin/env python3
"""List the stringData keys from a Secret-shaped YAML example.

CLI:
  parse_secret_yaml_example.py --example PATH

Stdout: one key per line.
Exit:   0 found, 1 stringData missing.

Used by Phase 16 to derive `secret.required_fields` for descriptor.yaml.
"""
import argparse, re, sys

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--example", required=True)
    args = p.parse_args()
    in_block = False
    keys = []
    with open(args.example, "r", encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            if not in_block:
                if re.match(r"^stringData\s*:\s*$", line):
                    in_block = True
                continue
            if line and not line.startswith(" "):
                break
            m = re.match(r"^\s+([A-Za-z0-9_.\-]+)\s*:", line)
            if m:
                keys.append(m.group(1))
    if not keys:
        print("parse_secret_yaml_example: no stringData section found", file=sys.stderr)
        return 1
    sys.stdout.write("\n".join(keys))
    return 0

if __name__ == "__main__":
    sys.exit(main())
