"""One canonical tenant UUID, read from one value, in every consumer.

`ingestion.reconcile.tenantId` is the canonical tenant. The deprecated
`global.tenantDefaultId` is still read while installs move off it, and the two
naming different tenants is refused rather than resolved — that disagreement is
exactly the drift that left data under a tenant nobody could read.

A warm install declares the key its existing Bronze rows and Airbyte objects
were created with in `legacyTenantKeyOverride`; only reconcile hears it.

Run: pytest charts/insight/tests
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]

CANONICAL = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
OTHER = "7a1e5c3b-9d2f-4b6a-8e0c-1f2a3b4c5d6e"

KEYCLOAK = [
    "--set", "keycloakConfig.enabled=true",
    "--set", "keycloakConfig.url=https://kc.example.test",
    "--set", "keycloakConfig.existingSecret=kc",
]

#: Template -> where it carries the canonical tenant.
CONSUMERS = {
    "templates/ingestion/reconcile-cron.yaml": ("INSIGHT_TENANT_ID",),
    "templates/keycloak-config-job.yaml": ("INSIGHT_TENANT_ID",),
    "templates/secrets.yaml": (
        "APP__gears__identity_resolution__config__tenant_default_id",
        "APP__gears__authenticator__config__idp__default_tenant_id",
    ),
}


@pytest.fixture(scope="session")
def chart_dependencies() -> None:
    result = subprocess.run(
        ["helm", "dependency", "update", str(CHART)], capture_output=True, text=True, timeout=300, check=False
    )
    assert result.returncode == 0, result.stderr


def _helm(template: str, overrides: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "helm", "template", "contract-test", str(CHART),
            "--values", str(CHART / "tests" / "values.yaml"),
            *KEYCLOAK, *overrides,
            "--show-only", template,
        ],
        capture_output=True, text=True, timeout=300, check=False,
    )


def _carried(template: str, overrides: list[str], names: tuple[str, ...]) -> dict[str, set[str]]:
    result = _helm(template, overrides)
    assert result.returncode == 0, result.stderr
    found: dict[str, set[str]] = {}
    for document in yaml.safe_load_all(result.stdout):
        for key, value in _entries(document or {}):
            if key in names:
                found.setdefault(key, set()).add(value)
    return found


def _entries(node: object) -> list[tuple[str, str]]:
    """Secret/ConfigMap data and container env, wherever the manifest nests them."""
    if isinstance(node, list):
        return [entry for item in node for entry in _entries(item)]
    if not isinstance(node, dict):
        return []
    entries = [
        (e["name"], e.get("value", "")) for e in node.get("env", []) if isinstance(e, dict) and "name" in e
    ]
    for block in ("data", "stringData"):
        if isinstance(node.get(block), dict):
            entries += [(k, v) for k, v in node[block].items() if isinstance(v, str)]
    for value in node.values():
        entries += _entries(value)
    return entries


@pytest.mark.parametrize(("template", "names"), CONSUMERS.items())
def test_every_consumer_hears_the_canonical_tenant(
    chart_dependencies: None, template: str, names: tuple[str, ...]
) -> None:
    found = _carried(template, ["--set", f"ingestion.reconcile.tenantId={CANONICAL}"], names)
    assert found == {name: {CANONICAL} for name in names}


@pytest.mark.parametrize(("template", "names"), CONSUMERS.items())
def test_the_deprecated_global_is_read_when_the_canonical_value_is_empty(
    chart_dependencies: None, template: str, names: tuple[str, ...]
) -> None:
    overrides = ["--set", "ingestion.reconcile.tenantId=", "--set", f"global.tenantDefaultId={CANONICAL}"]
    assert _carried(template, overrides, names) == {name: {CANONICAL} for name in names}


def test_two_different_tenants_are_refused(chart_dependencies: None) -> None:
    result = _helm(
        "templates/secrets.yaml",
        ["--set", f"ingestion.reconcile.tenantId={CANONICAL}", "--set", f"global.tenantDefaultId={OTHER}"],
    )
    assert result.returncode != 0
    assert "name different tenants" in result.stderr


@pytest.mark.parametrize("value", ["example-legacy", CANONICAL.upper()])
def test_a_tenant_that_is_not_a_lowercase_uuid_is_refused(chart_dependencies: None, value: str) -> None:
    result = _helm("templates/ingestion/reconcile-cron.yaml", ["--set", f"ingestion.reconcile.tenantId={value}"])
    assert result.returncode != 0
    assert "legacyTenantKeyOverride" in result.stderr


def test_a_fresh_install_gives_reconcile_no_legacy_key(chart_dependencies: None) -> None:
    found = _carried(
        "templates/ingestion/reconcile-cron.yaml",
        ["--set", f"ingestion.reconcile.tenantId={CANONICAL}"],
        ("INSIGHT_LEGACY_TENANT_KEY",),
    )
    assert found == {}


def test_a_warm_install_gives_reconcile_its_legacy_key(chart_dependencies: None) -> None:
    found = _carried(
        "templates/ingestion/reconcile-cron.yaml",
        [
            "--set", f"ingestion.reconcile.tenantId={CANONICAL}",
            "--set", "ingestion.reconcile.legacyTenantKeyOverride=example-legacy",
        ],
        ("INSIGHT_TENANT_ID", "INSIGHT_LEGACY_TENANT_KEY"),
    )
    assert found == {"INSIGHT_TENANT_ID": {CANONICAL}, "INSIGHT_LEGACY_TENANT_KEY": {"example-legacy"}}


@pytest.mark.parametrize("template", ["templates/secrets.yaml", "templates/keycloak-config-job.yaml"])
def test_the_legacy_key_reaches_nothing_but_reconcile(chart_dependencies: None, template: str) -> None:
    result = _helm(
        template,
        [
            "--set", f"ingestion.reconcile.tenantId={CANONICAL}",
            "--set", "ingestion.reconcile.legacyTenantKeyOverride=example-legacy",
        ],
    )
    assert result.returncode == 0, result.stderr
    assert "example-legacy" not in result.stdout
