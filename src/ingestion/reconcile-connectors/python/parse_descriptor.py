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
    def _unquote(s: str) -> str:
        # YAML quoted scalars: `image: "ghcr.io/...:tag"` must yield the
        # bare string, as yaml.safe_load does. Without this, dev-up.sh
        # passes a docker ref with literal quotes — `invalid reference
        # format`. Only strip a MATCHING surrounding pair.
        if len(s) >= 2 and s[0] == s[-1] and s[0] in ("'", '"'):
            return s[1:-1]
        return s

    def _strip_comment(s: str) -> str:
        # yaml.safe_load drops trailing ` # ...` comments; without this the
        # fallback returns `"0 2 * * *" # daily at 02:00 UTC` verbatim for
        # github-copilot's schedule — quotes and comment included — and
        # reconcile gets a bogus cron. A `#` only starts a comment outside
        # quotes and when preceded by whitespace (or at value start).
        if s[:1] in ("'", '"'):
            end = s.find(s[0], 1)
            if end != -1:
                rest = s[end + 1:]
                if rest == "" or (
                    rest[0] in " \t" and rest.lstrip(" \t").startswith("#")
                ):
                    return s[: end + 1]
            return s
        for i, ch in enumerate(s):
            if ch == "#" and (i == 0 or s[i - 1] in " \t"):
                return s[:i].rstrip()
        return s

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
        # Stack entries are (indent, container, parent, key). parent/key
        # locate the container inside its parent so the empty-dict
        # placeholder pushed for `key:` can be swapped for a list when the
        # first "- item" child line arrives.
        stack = [(0, root, None, None)]
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
                if list_target is None:
                    _, container, parent, key = stack[-1]
                    if parent is None or container != {}:
                        raise ValueError(
                            f"{path}: list item {line!r} does not follow a "
                            "key with an empty value; unsupported by the "
                            "fallback parser — install PyYAML"
                        )
                    list_target = parent[key] = []
                    stack[-1] = (indent, list_target, parent, key)
                list_target.append(_unquote(_strip_comment(line[2:].strip())))
                continue
            if ":" in line:
                k, _, v = line.partition(":")
                k = k.strip()
                v = _strip_comment(v.strip())
                if v == "":
                    # nested block: dict placeholder until the first child
                    # line shows it's a list ("- item" swaps it above).
                    cur[k] = {}
                    stack.append((indent + 2, cur[k], cur, k))
                    list_target = None
                elif v == "[]" or (v.startswith("[") and v.endswith("]")):
                    inner = v[1:-1].strip()
                    cur[k] = [_unquote(s.strip()) for s in inner.split(",") if s.strip()]
                else:
                    cur[k] = _unquote(v)
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
