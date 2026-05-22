#!/usr/bin/env python3
"""
discover-image-matrix.py — scan connector descriptors and emit a CI build matrix.

Reads every src/ingestion/connectors/*/*/descriptor.yaml and produces a JSON
list of {connector_dir, key, name, dockerfile, context, image} entries — one
per `descriptor.yaml.images.<key>` entry per ADR-0016.

When --changed is provided, the list is filtered to entries whose `context`
directory (relative to the connector dir) is a prefix of at least one changed
path, OR whose `dockerfile` path is itself among the changed paths. The
connector's `descriptor.yaml` itself is EXCLUDED from the trigger set — a
descriptor-only change must NOT re-fire image builds, otherwise the
bump-descriptors commit (which patches images.<key>.image) would re-trigger
the workflow in a loop.

When --all is set (legacy workflow_dispatch behaviour), every entry is emitted
regardless of changed paths.

Usage:
  discover-image-matrix.py
      --connectors-root src/ingestion/connectors
      [--changed FILE]          # file with one changed path per line (vs $BASE)
      [--all]                   # emit every entry, ignoring --changed

Stdout: JSON array. Exit: 0 always (empty array on no-match is valid).
"""
from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Iterable

try:
    import yaml
except ImportError:
    sys.stderr.write("ERROR: PyYAML is required (pip install pyyaml)\n")
    sys.exit(2)


def _iter_descriptors(connectors_root: Path) -> Iterable[Path]:
    """Yield every descriptor.yaml under the connectors root, sorted."""
    return sorted(connectors_root.glob("*/*/descriptor.yaml"))


def _norm(path: str) -> str:
    """Normalize a path: strip leading './', collapse double slashes."""
    return os.path.normpath(path).lstrip("./") if path else ""


def _entry_under(changed: list[str], entry_root: str, dockerfile_path: str, descriptor_path: str) -> bool:
    """Return True iff at least one changed path lives under entry_root
    (resolved relative to repo root) AND is not the descriptor.yaml,
    OR the Dockerfile itself is among the changed paths.
    """
    entry_root = entry_root.rstrip("/") + "/"
    for p in changed:
        if p == descriptor_path:
            continue  # excluded — loop-break for the bump commit
        if p == dockerfile_path:
            return True
        if p.startswith(entry_root):
            return True
    return False


def _load_descriptor(path: Path) -> dict | None:
    """Load and parse a descriptor; return None on parse error (with WARN)."""
    try:
        with open(path) as f:
            return yaml.safe_load(f) or {}
    except Exception as exc:  # noqa: BLE001
        sys.stderr.write(f"WARN: cannot parse {path}: {exc}\n")
        return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--connectors-root", required=True,
                        help="path to the connectors root, e.g. src/ingestion/connectors")
    parser.add_argument("--changed", default=None,
                        help="file containing one changed path per line; if absent, --all is implied")
    parser.add_argument("--all", action="store_true",
                        help="emit every entry, ignoring changed-path filter")
    args = parser.parse_args()

    connectors_root = Path(args.connectors_root)
    if not connectors_root.is_dir():
        sys.stderr.write(f"ERROR: {connectors_root} is not a directory\n")
        return 2

    # Build absolute set of changed paths once; treat empty stdin / missing
    # file as "no changes" which combined with not --all yields an empty
    # matrix — let the workflow's `if needs.discover-images.outputs.any`
    # gate handle it.
    changed: list[str] = []
    if args.changed and not args.all:
        try:
            with open(args.changed) as f:
                changed = [ln.strip() for ln in f if ln.strip()]
        except FileNotFoundError:
            changed = []

    out = []
    for desc_path in _iter_descriptors(connectors_root):
        d = _load_descriptor(desc_path)
        if d is None:
            continue
        images = d.get("images") or {}
        if not isinstance(images, dict):
            sys.stderr.write(f"WARN: {desc_path}: images must be a map (per ADR-0016); skipping\n")
            continue
        connector_dir = str(desc_path.parent)
        for key, entry in images.items():
            if not isinstance(entry, dict):
                sys.stderr.write(f"WARN: {desc_path}: images.{key} must be a map; skipping\n")
                continue
            required = ("name", "dockerfile", "context")
            missing = [f for f in required if not entry.get(f)]
            if missing:
                sys.stderr.write(
                    f"WARN: {desc_path}: images.{key} missing required fields "
                    f"{missing}; skipping\n"
                )
                continue
            name = entry["name"]
            dockerfile_rel = entry["dockerfile"]
            context_rel = entry["context"]
            image = entry.get("image", "") or ""
            # Resolve relative paths.
            entry_root = os.path.normpath(os.path.join(connector_dir, context_rel))
            dockerfile_path = os.path.normpath(os.path.join(connector_dir, dockerfile_rel))
            descriptor_path = os.path.normpath(str(desc_path))

            if args.all or not args.changed:
                fires = True
            else:
                fires = _entry_under(changed, entry_root, dockerfile_path, descriptor_path)

            if fires:
                out.append({
                    "connector_dir": connector_dir,
                    "key": key,
                    "name": name,
                    "dockerfile": dockerfile_rel,
                    "context": context_rel,
                    "image": image,
                })

    # Compact JSON on stdout.
    print(json.dumps(out, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    sys.exit(main())
