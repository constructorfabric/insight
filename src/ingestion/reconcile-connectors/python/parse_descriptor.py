#!/usr/bin/env python3
"""Read a dotted-path field from a connector descriptor.yaml.

CLI:
  parse_descriptor.py --descriptor PATH --field DOTTED.PATH

Examples:
  --field version                 -> prints scalar
  --field schedule                -> prints scalar
  --field secret.required_fields  -> prints list as one-name-per-line

Stdout: scalar string OR newline-separated list.
Exit:   0 found, 1 not found, 2 PyYAML missing.

Requires PyYAML. Every caller runs where it is present: the reconcile
toolbox image ships it transitively via `dbt-clickhouse`, and CI installs
it explicitly (build-images.yml). A hand-rolled fallback parser used to
cover the old host-python `dev-up.sh` path; that path is retired, so the
fallback was removed in favour of the library parser -- no silent
divergence from `yaml.safe_load`.
"""
import argparse
import sys

try:
    import yaml
except ImportError:
    sys.stderr.write(
        "parse_descriptor.py requires PyYAML, which is not importable.\n"
        "Install it with: python3 -m pip install pyyaml\n"
        "(The reconcile toolbox image ships it via dbt-clickhouse; "
        "CI installs it explicitly.)\n"
    )
    sys.exit(2)


def _read_yaml(path: str):
    with open(path, "r", encoding="utf-8") as f:
        return yaml.safe_load(f) or {}


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
    val = _walk(_read_yaml(args.descriptor), args.field)
    if val is None:
        return 1
    if isinstance(val, list):
        sys.stdout.write("\n".join(map(str, val)))
    else:
        sys.stdout.write(str(val))
    return 0


if __name__ == "__main__":
    sys.exit(main())
