#!/usr/bin/env python3
"""Validate a K8s Secret's stringData against descriptor.yaml.secret.required_fields.

CLI:
  validate_secret.py --descriptor PATH --secret-stringdata PATH

PATH for --secret-stringdata: a JSON file containing the stringData object
(typically produced by `kubectl get secret X -o json | jq .stringData`).

Exit:
  0   valid (every required field present and non-empty)
  2   invalid; prints `MISSING: <field-name>` to stderr (one line per
      missing field) and the FIRST missing field name on stdout (no
      trailing newline) for easy shell capture.

Per ADR-0007, descriptor is the single source of truth for required_fields.
"""
import argparse, json, sys

try:
    import yaml as _yaml
    def _read_yaml_required_fields(path: str):
        with open(path, "r", encoding="utf-8") as f:
            d = _yaml.safe_load(f) or {}
        return (d.get("secret") or {}).get("required_fields") or []
except ImportError:
    def _read_yaml_required_fields(path: str):
        # Minimal YAML reader for the `secret.required_fields` shape:
        #   secret:
        #     required_fields:
        #       - name1
        #       - name2
        out = []
        in_block = False
        with open(path, "r", encoding="utf-8") as f:
            for raw in f:
                line = raw.rstrip("\n")
                if not in_block:
                    if line.strip() == "secret:":
                        in_block = "secret"
                    continue
                if in_block == "secret":
                    if line.startswith("  required_fields:"):
                        in_block = "list"
                        continue
                    if line and not line.startswith(" "):
                        in_block = False
                elif in_block == "list":
                    stripped = line.strip()
                    if stripped.startswith("- "):
                        out.append(stripped[2:].strip())
                    elif line and not line.startswith("    "):
                        in_block = False
        return out

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--descriptor", required=True)
    p.add_argument("--secret-stringdata", required=True)
    args = p.parse_args()
    required = _read_yaml_required_fields(args.descriptor)
    with open(args.secret_stringdata, "r", encoding="utf-8") as f:
        present = json.load(f) or {}
    missing = [k for k in required if k not in present or present[k] in (None, "")]
    if missing:
        for m in missing:
            print(f"MISSING: {m}", file=sys.stderr)
        sys.stdout.write(missing[0])
        return 2
    return 0

if __name__ == "__main__":
    sys.exit(main())
