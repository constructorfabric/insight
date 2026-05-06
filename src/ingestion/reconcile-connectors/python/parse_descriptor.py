#!/usr/bin/env python3
"""Read a dotted-path field from a connector descriptor.yaml.

CLI:
  parse_descriptor.py --descriptor PATH --field DOTTED.PATH

Examples:
  --field version              → prints scalar
  --field schedule             → prints scalar
  --field secret.required_fields -> prints list as one-name-per-line

Stdout: scalar string OR newline-separated list.
Exit:   0 found, 1 not found.
"""
import argparse, sys

try:
    import yaml as _yaml
    def _read_yaml(path: str):
        with open(path, "r", encoding="utf-8") as f:
            return _yaml.safe_load(f) or {}
except ImportError:
    def _read_yaml(path: str):
        with open(path, "r", encoding="utf-8") as f:
            text = f.read()
        # Minimal hand-rolled YAML reader supporting:
        #   key: scalar
        #   key:
        #     subkey: scalar
        #     listkey:
        #       - item1
        #       - item2
        root = {}
        stack = [(0, root)]
        list_target = None
        for raw in text.splitlines():
            if not raw.strip() or raw.lstrip().startswith("#"): continue
            indent = len(raw) - len(raw.lstrip())
            line = raw.strip()
            # pop stack to current indent
            while stack and stack[-1][0] > indent:
                stack.pop(); list_target = None
            cur = stack[-1][1]
            if line.startswith("- "):
                if list_target is None: continue
                list_target.append(line[2:].strip())
                continue
            if ":" in line:
                k, _, v = line.partition(":")
                k = k.strip(); v = v.strip()
                if v == "":
                    # nested object or list
                    cur[k] = {}
                    stack.append((indent + 2, cur[k]))
                    list_target = None
                    # peek next significant line — if it starts with "- " set list_target
                    # we let the next iteration set list_target lazily by replacing dict with list.
                elif v == "[]" or v.startswith("[") and v.endswith("]"):
                    inner = v[1:-1].strip()
                    cur[k] = [s.strip() for s in inner.split(",") if s.strip()]
                else:
                    cur[k] = v
        return root

def _walk(obj, dotted: str):
    cur = obj
    for part in dotted.split("."):
        if isinstance(cur, dict) and part in cur:
            cur = cur[part]
        else:
            return None
    return cur

def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--descriptor", required=True)
    p.add_argument("--field", required=True)
    args = p.parse_args()
    doc = _read_yaml(args.descriptor)
    val = _walk(doc, args.field)
    if val is None:
        return 1
    if isinstance(val, list):
        sys.stdout.write("\n".join(map(str, val)))
    else:
        sys.stdout.write(str(val))
    return 0

if __name__ == "__main__":
    sys.exit(main())
