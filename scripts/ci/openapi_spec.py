#!/usr/bin/env python3
"""Generate / drift-check a committed OpenAPI spec against a service's live spec.

Shared by both backend services. The live spec is whatever the service emits,
captured to a file:
  - analytics: the offline `analytics openapi` subcommand
  - identity: the served `GET /openapi.json`, dumped by its integration test

This is the whole gate — one script, no shell wrapper, pure stdlib:

    python3 scripts/ci/openapi_spec.py check  --file <committed> --live-file <dump>
    python3 scripts/ci/openapi_spec.py update --file <committed> --live-file <dump>

`--live-file` (required) is a saved OpenAPI JSON dump; `--file` is the committed
doc to drift-check (`check`) or rewrite (`update`) and defaults to analytics's
doc relative to the repo root, so the script runs from any working directory.
"""

from __future__ import annotations

import argparse
import difflib
import json
import sys
from pathlib import Path

# scripts/ci/openapi_spec.py -> scripts/ci -> scripts -> repo root.
_REPO_ROOT = Path(__file__).resolve().parents[2]
# The committed doc defaults to analytics's (a code constant, not
# operator-tunable input); identity overrides it with --file. The live dump is
# always caller-supplied (--live-file), so it has no default.
DEFAULT_SPEC_FILE = _REPO_ROOT / "docs/components/backend/analytics/openapi.json"


def normalize(doc: object) -> str:
    """Canonical on-disk form: sorted keys, 2-space indent, trailing newline.

    Sorting keys makes the comparison independent of the emitter's key order
    (serde_json / System.Text.Json iterate differently), so ``check`` is stable
    run-to-run and ``update`` produces a minimal, review-friendly diff.
    """
    return json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def _load_live(live_file: str) -> tuple[str | None, str | None]:
    """Return (normalized live spec, source path), or (None, None) on a missing
    file (after printing an error)."""
    live_path = Path(live_file)
    if not live_path.exists():
        print(
            f"ERROR: {live_path} not found — pass --live-file pointing at a "
            f"generated OpenAPI dump (e.g. `analytics openapi > dump.json`, or "
            f"the identity integration test's IDENTITY_OPENAPI_DUMP output)",
            file=sys.stderr,
        )
        return None, None
    return normalize(json.loads(live_path.read_text(encoding="utf-8"))), str(live_path)


def main() -> int:
    p = argparse.ArgumentParser(
        description="Generate / drift-check a committed OpenAPI spec."
    )
    p.add_argument(
        "mode",
        choices=["check", "update"],
        help="check: exit 2 on drift; update: rewrite the committed doc",
    )
    p.add_argument(
        "--live-file",
        required=True,
        help="generated OpenAPI JSON dump to drift-check against (required)",
    )
    p.add_argument("--file", default=str(DEFAULT_SPEC_FILE), help="committed spec path")
    args = p.parse_args()

    live, source = _load_live(args.live_file)
    if live is None:
        return 2
    path = Path(args.file)

    if args.mode == "update":
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(live, encoding="utf-8")
        print(f"wrote {path} ({len(live.splitlines())} lines) from {source}")
        return 0

    # check
    if not path.exists():
        print(
            f"ERROR: {path} does not exist — run "
            f"`python3 scripts/ci/openapi_spec.py update` to create it",
            file=sys.stderr,
        )
        return 2
    committed = path.read_text(encoding="utf-8")
    if committed == live:
        print(f"OK: {path} matches the live spec ({source})")
        return 0

    sys.stdout.writelines(
        difflib.unified_diff(
            committed.splitlines(keepends=True),
            live.splitlines(keepends=True),
            fromfile=f"{path} (committed)",
            tofile=f"{source} (live)",
        )
    )
    print(
        f"\nERROR: {path} is STALE vs the live spec.\n"
        f"Regenerate the live dump, run `python3 scripts/ci/openapi_spec.py update "
        f"--file {path} --live-file <dump>`, and commit {path}.",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
