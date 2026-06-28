"""Collect coverage-gate inputs while analytics-api is already up.

The metric-coverage and openapi-spec-drift gates used to each boot their OWN
throwaway analytics-api just to read the catalog / OpenAPI spec. Instead,
snapshot both here — during the e2e run, where the API is already live — into
`.artifacts/`, so the CI gates analyse plain files with no Docker boot:

  • openapi.live.json   ← GET /openapi.json          (openapi-spec-drift gate)
  • catalog_metrics.json ← POST /v1/catalog/get_metrics (metric-coverage gate)

(The endpoint-coverage ledger, observed_endpoints.json, is written separately
by conftest.pytest_sessionfinish.)

Uses a RAW httpx client — NOT `analytics_api.client()`, which carries the
endpoint-coverage recording hook — so these infrastructure calls do NOT count
as suite coverage in the endpoint ledger.
"""

from __future__ import annotations

import json
from pathlib import Path

import httpx

from lib.analytics_api import AnalyticsApiProcess
from lib.config import TENANT_HEADER, TEST_TENANT_ID

# api/ -> e2e/ ; same dir conftest.pytest_sessionfinish writes the ledger to.
_ARTIFACTS = Path(__file__).resolve().parents[1] / ".artifacts"


def test_collect_coverage_artifacts(analytics_api: AnalyticsApiProcess) -> None:
    """Snapshot the live OpenAPI spec + metric catalog for the CI gates.

    Doubles as a smoke check: the spec must declare paths and the catalog must
    return metrics.
    """
    _ARTIFACTS.mkdir(exist_ok=True)
    # Un-hooked client: collection traffic must NOT land in the endpoint ledger.
    with httpx.Client(
        base_url=analytics_api.base_url,
        timeout=30.0,
        headers={TENANT_HEADER: str(TEST_TENANT_ID)},
    ) as c:
        spec = c.get("/openapi.json")
        spec.raise_for_status()
        catalog = c.post("/v1/catalog/get_metrics", json={})
        catalog.raise_for_status()

    spec_doc = spec.json()
    catalog_doc = catalog.json()
    assert spec_doc.get("paths"), "GET /openapi.json returned no paths"
    assert catalog_doc.get("metrics"), "POST /v1/catalog/get_metrics returned no metrics"

    (_ARTIFACTS / "openapi.live.json").write_text(
        json.dumps(spec_doc, indent=2) + "\n", encoding="utf-8"
    )
    (_ARTIFACTS / "catalog_metrics.json").write_text(
        json.dumps(catalog_doc, indent=2) + "\n", encoding="utf-8"
    )
