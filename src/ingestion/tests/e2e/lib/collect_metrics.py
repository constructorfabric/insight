#!/usr/bin/env python3
"""Snapshot the metric catalog from a running analytics.

NOT a pytest test — a plain script, run (via subprocess) by conftest's
`analytics` fixture while the suite's analytics is up, so the
metric-coverage gate can read a file with no second app boot. Writes
``catalog_metrics.json`` (← POST /v1/catalog/get_metrics) into ``--out-dir``.

Standalone:
    python3 lib/collect_metrics.py \
        --url http://127.0.0.1:8081 --out-dir .artifacts \
        --bearer "<signed gateway JWT>"
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def collect(base_url: str, out_dir: str | Path, bearer: str | None) -> None:
    """Fetch the metric catalog and write it to ``out_dir``.

    Fail-fast (raises) if the response is empty — a missing/empty artifact would
    otherwise surface only as a confusing downstream gate failure.
    """
    import httpx  # local import: keeps this importable without httpx

    headers: dict[str, str] = {}
    if bearer:
        # Analytics verifies the signed gateway JWT (NGINX_BFF R1); its
        # `tenant_id` claim scopes the tenant-gated catalog read.
        headers["Authorization"] = f"Bearer {bearer}"
    with httpx.Client(base_url=base_url, timeout=30.0, headers=headers) as c:
        catalog = c.post("/v1/catalog/get_metrics", json={})
        catalog.raise_for_status()

    catalog_doc = catalog.json()
    if not (isinstance(catalog_doc, dict) and catalog_doc.get("metrics")):
        raise SystemExit(f"collect: POST {base_url}/v1/catalog/get_metrics returned no metrics")

    out = Path(out_dir)
    out.mkdir(parents=True, exist_ok=True)
    (out / "catalog_metrics.json").write_text(json.dumps(catalog_doc, indent=2) + "\n", encoding="utf-8")
    print(f"collected catalog_metrics.json ({len(catalog_doc['metrics'])} metrics) -> {out}")  # noqa: T201


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description="Snapshot the analytics metric catalog for the coverage gate.")
    p.add_argument("--url", required=True, help="analytics base URL")
    p.add_argument("--out-dir", required=True, help="directory to write the artifact into")
    p.add_argument("--bearer", help="signed gateway JWT (Authorization: Bearer); the catalog read is tenant-gated")
    args = p.parse_args(argv)
    collect(args.url, args.out_dir, args.bearer)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
