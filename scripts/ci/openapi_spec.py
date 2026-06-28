#!/usr/bin/env python3
"""Generate / drift-check the committed analytics-api OpenAPI spec.

The live contract is the in-process registry served at ``GET /openapi.json``
(``src/backend/services/analytics-api/src/api/mod.rs``). This script fetches it
and either rewrites the committed copy (``write``) or fails on any drift
(``check``), so the doc can never silently fall behind the router.

Host deps: ``httpx`` only (stdlib otherwise). URL mode — needs a running
analytics-api reachable at ``$ANALYTICS_API_URL``; ``scripts/ci/openapi_spec.sh``
boots a throwaway one.

    ANALYTICS_API_URL=http://localhost:18081 python3 scripts/ci/openapi_spec.py check
    ANALYTICS_API_URL=http://localhost:18081 python3 scripts/ci/openapi_spec.py write
"""

from __future__ import annotations

import argparse
import difflib
import json
import os
import sys
from pathlib import Path

# Canonical location of the committed spec — a fixed repo artifact path (not an
# operator-tunable input), so it is a constant default, overridable with --file.
DEFAULT_SPEC_FILE = "docs/components/backend/analytics-api/openapi.json"
SPEC_ROUTE = "/openapi.json"


def normalize(doc: object) -> str:
    """Canonical on-disk form: sorted keys, 2-space indent, trailing newline.

    Sorting keys makes the comparison independent of the registry's emission
    order (the toolkit iterates a ``DashMap``), so ``check`` is stable run-to-run
    and ``write`` produces a minimal, review-friendly diff.
    """
    return json.dumps(doc, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def fetch_live_spec(base_url: str, tenant_id: str | None) -> object:
    import httpx  # local import: keep `normalize` importable without httpx

    headers: dict[str, str] = {}
    if tenant_id:
        # /openapi.json is public, but sending the header is harmless and keeps
        # the call uniform with the rest of the rig.
        headers["X-Insight-Tenant-Id"] = tenant_id
    with httpx.Client(base_url=base_url, timeout=30.0, headers=headers) as c:
        r = c.get(SPEC_ROUTE)
        r.raise_for_status()
        return r.json()


def main() -> int:
    p = argparse.ArgumentParser(
        description="Generate / drift-check the analytics-api OpenAPI spec."
    )
    p.add_argument("mode", choices=["check", "write"])
    p.add_argument("--url", help="analytics-api base URL (default: $ANALYTICS_API_URL)")
    p.add_argument(
        "--file",
        default=DEFAULT_SPEC_FILE,
        help=f"committed spec path (default: {DEFAULT_SPEC_FILE})",
    )
    p.add_argument(
        "--tenant",
        help="X-Insight-Tenant-Id header (default: $ANALYTICS_TENANT_ID; "
        "optional — the route is public)",
    )
    p.add_argument(
        "--live-file",
        help="read the live spec from this saved GET /openapi.json (the "
        "openapi.live.json artifact the e2e run collects) instead of fetching — "
        "lets the CI gate run with no analytics-api boot",
    )
    args = p.parse_args()

    # Live spec source: a collected file (CI gate — no boot) or a live fetch.
    if args.live_file:
        live = normalize(json.loads(Path(args.live_file).read_text(encoding="utf-8")))
        source = args.live_file
    else:
        # Required input — fail fast, no silent fallback (code-conventions §No defaults).
        base_url = args.url or os.environ.get("ANALYTICS_API_URL")
        if not base_url:
            print(
                "ERROR: pass --live-file <openapi.live.json>, or --url / "
                "$ANALYTICS_API_URL to fetch a running analytics-api",
                file=sys.stderr,
            )
            return 2
        tenant = args.tenant or os.environ.get("ANALYTICS_TENANT_ID")
        live = normalize(fetch_live_spec(base_url, tenant))
        source = f"{base_url}{SPEC_ROUTE}"

    path = Path(args.file)

    if args.mode == "write":
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(live, encoding="utf-8")
        print(f"wrote {path} ({len(live.splitlines())} lines) from {source}")
        return 0

    # check
    if not path.exists():
        print(
            f"ERROR: {path} does not exist — run `bash scripts/ci/openapi_spec.sh "
            f"update` to create it",
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
        f"Regenerate it:  bash scripts/ci/openapi_spec.sh update\n"
        f"(then commit the updated {path})",
        file=sys.stderr,
    )
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
