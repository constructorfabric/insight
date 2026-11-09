"""One tenant, set in one value, heard by every consumer.

`authenticator.oidc.defaultTenantId` is the install's tenant. Every key that
used to hold a copy of it is refused rather than ignored: an install still
carrying one would otherwise keep a second tenant nobody reads, or lose the
one it meant on upgrade.

Run: pytest charts/insight/tests
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]

TENANT = "0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f"
TENANT_KEY = "authenticator.oidc.defaultTenantId"

KEYCLOAK = [
    "--set", "keycloakConfig.enabled=true",
    "--set", "keycloakConfig.url=https://kc.example.test",
]  # fmt: skip

#: Template -> the manifest keys in it that carry the tenant.
CONSUMERS = {
    "templates/ingestion/reconcile-cron.yaml": ("INSIGHT_TENANT_ID",),
    "templates/keycloak-config-job.yaml": ("INSIGHT_TENANT_ID",),
    "templates/secrets.yaml": (
        "APP__gears__authenticator__config__idp__default_tenant_id",
        "APP__gears__identity_resolution__config__tenant_default_id",
    ),
}

REMOVED_KEYS = ["global.tenantDefaultId", "ingestion.reconcile.tenantId", "identityResolution.seed.tenantDefaultId"]


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
    )  # fmt: skip


def _entries(node: object) -> list[tuple[str, str]]:
    """Secret data and container env, wherever the manifest nests them."""
    if isinstance(node, list):
        return [entry for item in node for entry in _entries(item)]
    if not isinstance(node, dict):
        return []

    entries = [(e["name"], e.get("value", "")) for e in node.get("env", []) if isinstance(e, dict) and "name" in e]
    for block in ("data", "stringData"):
        if isinstance(node.get(block), dict):
            entries += [(k, v) for k, v in node[block].items() if isinstance(v, str)]
    for value in node.values():
        entries += _entries(value)
    return entries


def _carried(template: str, overrides: list[str], names: tuple[str, ...]) -> dict[str, set[str]]:
    result = _helm(template, overrides)
    assert result.returncode == 0, result.stderr

    found: dict[str, set[str]] = {}
    for document in yaml.safe_load_all(result.stdout):
        for key, value in _entries(document or {}):
            if key in names:
                found.setdefault(key, set()).add(value)
    return found


@pytest.mark.parametrize(("template", "names"), CONSUMERS.items())
def test_every_consumer_hears_the_one_tenant(chart_dependencies: None, template: str, names: tuple[str, ...]) -> None:
    found = _carried(template, ["--set", f"{TENANT_KEY}={TENANT}"], names)

    assert found == {name: {TENANT} for name in names}, template


@pytest.mark.parametrize("removed", REMOVED_KEYS)
def test_a_key_that_used_to_carry_the_tenant_fails_the_render(chart_dependencies: None, removed: str) -> None:
    result = _helm("templates/secrets.yaml", ["--set", f"{removed}={TENANT}"])

    assert result.returncode != 0, f"a leftover {removed} must fail the render"
    assert f"{removed} is removed" in result.stderr, result.stderr
    assert TENANT_KEY in result.stderr, result.stderr


@pytest.mark.parametrize(
    "value", ["example-legacy", TENANT.upper(), f" {TENANT}"], ids=["a slug", "upper case", "padded"]
)
def test_a_tenant_that_is_not_a_lowercase_uuid_fails_the_render(chart_dependencies: None, value: str) -> None:
    result = _helm("templates/ingestion/reconcile-cron.yaml", ["--set-string", f"{TENANT_KEY}={value}"])

    assert result.returncode != 0, f"should refuse {value!r}"
    assert "must be a lowercase UUID" in result.stderr, result.stderr


@pytest.mark.parametrize("template", list(CONSUMERS))
def test_an_install_without_a_tenant_fails_the_render(chart_dependencies: None, template: str) -> None:
    result = _helm(template, ["--set", f"{TENANT_KEY}="])

    assert result.returncode != 0, f"{template} rendered without a tenant"
    assert f"{TENANT_KEY} is required" in result.stderr, result.stderr
