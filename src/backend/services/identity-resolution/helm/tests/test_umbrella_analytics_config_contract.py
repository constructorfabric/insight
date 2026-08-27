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


def _analytics_secret(manifests: str) -> dict:
    docs = [doc for doc in yaml.safe_load_all(manifests) if isinstance(doc, dict)]
    secrets = [
        doc for doc in docs if doc.get("kind") == "Secret" and doc["metadata"]["name"] == "insight-analytics-config"
    ]
    assert len(secrets) == 1, f"expected one analytics config Secret, got {len(secrets)}"

    return secrets[0]["stringData"]


@pytest.mark.parametrize(
    ("extra", "expected"), [((), False), (("--set", "analytics.metricCatalog.tenantMetricsEnabled=true"), True)]
)
def test_tenant_metrics_are_opt_in_per_installation(umbrella_deps, extra: tuple[str, ...], expected: bool) -> None:
    code, out, err = render(umbrella_deps, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}", *extra)
    assert code == 0, err

    assert _analytics_config(out)["metric_catalog"]["tenant_metrics_enabled"] is expected
    assert _analytics_config(out)["external_sources"] == []


def test_analytics_visibility_policy_reuses_identity_resolution_setting(umbrella_deps) -> None:
    code, out, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "identityResolution.visibilityPolicy=flat",
    )
    assert code == 0, err

    key = "APP__gears__analytics__config__visibility_policy"
    assert _analytics_secret(out)[key] == "flat"


def test_external_source_registry_reaches_analytics_config(umbrella_deps) -> None:
    code, out, err = render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "analytics.externalSources[0].id=github-main",
        "--set",
        "analytics.externalSources[0].provider=github",
        "--set",
        "analytics.externalSources[0].webBaseUrl=https://github.example.com/",
    )
    assert code == 0, err

    assert _analytics_config(out)["external_sources"] == [
        {
            "id": "github-main",
            "provider": "github",
            "web_base_url": "https://github.example.com/",
        }
    ]
