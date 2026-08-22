from __future__ import annotations

import pytest
import yaml
from conftest import TENANT, UMBRELLA_BASE, render


def _analytics_config(manifests: str) -> dict:
    docs = [doc for doc in yaml.safe_load_all(manifests) if isinstance(doc, dict)]
    configs = [
        doc
        for doc in docs
        if doc.get("kind") == "ConfigMap" and doc["metadata"]["name"].endswith("-analytics-gears-config")
    ]
    assert len(configs) == 1, f"expected one analytics gears ConfigMap, got {len(configs)}"

    host = yaml.safe_load(configs[0]["data"]["analytics.yaml"])
    return host["gears"]["analytics"]["config"]


@pytest.mark.parametrize(
    ("extra", "expected"), [((), False), (("--set", "analytics.metricCatalog.tenantMetricsEnabled=true"), True)]
)
def test_tenant_metrics_are_opt_in_per_installation(umbrella_deps, extra: tuple[str, ...], expected: bool) -> None:
    code, out, err = render(umbrella_deps, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}", *extra)
    assert code == 0, err

    assert _analytics_config(out)["metric_catalog"]["tenant_metrics_enabled"] is expected
