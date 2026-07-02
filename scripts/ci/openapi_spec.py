#!/usr/bin/env python3
"""Generate / drift-check the committed analytics-api OpenAPI spec.

The live contract is the in-process registry served at ``GET /openapi.json``
(``src/backend/services/analytics-api/src/api/mod.rs``). This is the whole gate —
one script, no shell wrapper:

    python3 scripts/ci/openapi_spec.py check     # exit 2 + diff if the doc drifted
    python3 scripts/ci/openapi_spec.py update     # rewrite the committed doc

Live-spec source, in precedence order:
  --url / $ANALYTICS_API_URL   fetch a running analytics-api (needs httpx)
  --live-file <path>           a saved GET /openapi.json
  (default)                    the openapi.live.json the e2e run collects into
                               src/ingestion/tests/e2e/.artifacts/ — run
                               ``./e2e.sh test`` first (CI passes the downloaded
                               artifact explicitly).

Pure stdlib unless fetching (--url). Paths default relative to the repo root, so
it runs from any working directory.
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import sys
from pathlib import Path

# scripts/ci/openapi_spec.py -> scripts/ci -> scripts -> repo root.
_REPO_ROOT = Path(__file__).resolve().parents[2]
# Fixed repo artifact paths (code constants, not operator-tunable input) —
# overridable with --file / --live-file.
DEFAULT_SPEC_FILE = _REPO_ROOT / "docs/components/backend/analytics-api/openapi.json"
DEFAULT_LIVE_FILE = _REPO_ROOT / "src/ingestion/tests/e2e/.artifacts/openapi.live.json"
SPEC_ROUTE = "/openapi.json"


def normalize(doc: object) -> str:
    """Canonical on-disk form: sorted keys, 2-space indent, trailing newline.

    Sorting keys makes the comparison independent of the registry's emission
    order (the toolkit iterates a ``DashMap``), so ``check`` is stable run-to-run
    and ``update`` produces a minimal, review-friendly diff.
    """
    return json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def fetch_live_spec(base_url: str, tenant_id: str | None) -> object:
    import httpx  # local import: keep the file-only path importable without httpx

    headers: dict[str, str] = {}
    if tenant_id:
        # /openapi.json is public, but sending the header is harmless and keeps
        # the call uniform with the rest of the rig.
        headers["X-Insight-Tenant-Id"] = tenant_id
    with httpx.Client(base_url=base_url, timeout=30.0, headers=headers) as c:
        r = c.get(SPEC_ROUTE)
        r.raise_for_status()
        return r.json()


def _load_live(args: argparse.Namespace) -> tuple[str | None, str | None]:
    """Return (normalized live spec, human-readable source), or (None, None) on a
    missing file (after printing an error)."""
    url = args.url or os.environ.get("ANALYTICS_API_URL")
    if url:
        tenant = args.tenant or os.environ.get("ANALYTICS_TENANT_ID")
        return normalize(fetch_live_spec(url, tenant)), f"{url}{SPEC_ROUTE}"
    live_path = Path(args.live_file)
    if not live_path.exists():
        print(
            f"ERROR: {live_path} not found — run `./e2e.sh test` first (it collects "
            f"the live spec), or pass --url / $ANALYTICS_API_URL to fetch a running API",
            file=sys.stderr,
        )
        return None, None
    return normalize(json.loads(live_path.read_text(encoding="utf-8"))), str(live_path)


def main() -> int:
    p = argparse.ArgumentParser(
        description="Generate / drift-check the analytics-api OpenAPI spec."
    )
    p.add_argument(
        "mode",
        choices=["check", "update"],
        help="check: exit 2 on drift; update: rewrite the committed doc",
    )
    p.add_argument("--url", help="fetch a running analytics-api (default: $ANALYTICS_API_URL)")
    p.add_argument(
        "--live-file",
        default=str(DEFAULT_LIVE_FILE),
        help="saved GET /openapi.json to read when not fetching "
        "(default: the e2e-collected artifact)",
    )
    p.add_argument("--file", default=str(DEFAULT_SPEC_FILE), help="committed spec path")
    p.add_argument(
        "--tenant",
        help="X-Insight-Tenant-Id header (default: $ANALYTICS_TENANT_ID; "
        "optional — the route is public)",
    )
    args = p.parse_args()

    live, source = _load_live(args)
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
        f"\nERROR: {path} is STALE vs the live analytics-api router.\n"
        f"Regenerate it:  ./e2e.sh test && python3 scripts/ci/openapi_spec.py update\n"
        f"(then commit the updated {path})",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
