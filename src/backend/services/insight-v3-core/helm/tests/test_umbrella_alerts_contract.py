"""Umbrella render-contract for the alerts half of insight-v3-core.

The facts here are invisible in a diff of either chart alone. The destinations
an operator lists in values become environment keys, and the service lowercases
every environment key before it reads it, so a name with an uppercase letter is
served under a spelling nobody configured; the render has to refuse it. Under
`credentials.autoGenerate: false` the whole config Secret is composed outside
the chart, so a destinations map in values renders nothing and exits 0 — the
service then starts with alerts on and nowhere to notify. Both guards live
outside the credential-mode body on purpose, and a test inside that body would
fall silent in exactly the mode that most needs it.

Renders the umbrella from a synthetic values set, never from a gitops overlay:
an overlay is free to change what it enables, and a contract test that follows
it stops asserting silently instead of failing.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve()
REPO_ROOT = HERE.parents[6]
UMBRELLA = REPO_ROOT / "charts" / "insight"

TENANT = "3e1d5a65-434c-95b4-8c1b-eb8f53a39bab"
WEBHOOK = "https://discord.example.test/api/webhooks/1/x"

# Minimum viable umbrella install with insight-v3-core deployed.
BASE = {
    "global.insightV3Core.deploy": "true",
    "global.tenantDefaultId": TENANT,
    "identityResolution.deploy": "true",
    "clickhouse.host": "ch",
    "clickhouse.username": "u",
    "clickhouse.database": "insight",
    "mariadb.host": "m",
    "mariadb.username": "insight",
    "mariadb.database": "insight",
    "redis.host": "redis",
    "redpanda.brokers": "rp:9092",
    "ingestion.reconcile.tenantId": "default",
    "authenticator.oidc.issuerUrl": "https://idp",
    "authenticator.oidc.clientId": "c",
    "authenticator.oidc.clientSecret": "s",
    "authenticator.oidc.redirectUri": "https://x/cb",
    "authenticator.oidc.sourceType": "ms-entra",
}
ALERTS_ON = {"insightV3Core.alerts.enabled": "true", "insightV3Core.alerts.redis.host": "redis"}
GITOPS = {"credentials.autoGenerate": "false", "credentials.deploymentMode": "gitops"}

DESTINATIONS_PREFIX = "APP__gears__insight_v3_core__config__alerts__destinations__"


def destination(name: str) -> dict[str, str]:
    return {
        f"insightV3Core.alerts.destinations.{name}.provider": "discord",
        f"insightV3Core.alerts.destinations.{name}.webhook_url": WEBHOOK,
    }


@pytest.fixture(scope="session")
def umbrella() -> Path:
    """Vendor the subcharts once per session; every render here needs them."""
    done = subprocess.run(
        ["helm", "dependency", "update", str(UMBRELLA)], capture_output=True, text=True, timeout=300, check=False
    )
    assert done.returncode == 0, done.stderr
    return UMBRELLA


def render(umbrella: Path, overrides: dict[str, str]) -> subprocess.CompletedProcess[str]:
    args = ["helm", "template", "contract-test", str(umbrella)]
    for key, value in {**BASE, **overrides}.items():
        args += ["--set", f"{key}={value}"]
    return subprocess.run(args, capture_output=True, text=True, timeout=300, check=False)


def rendered(umbrella: Path, **overrides: str) -> list[dict]:
    done = render(umbrella, overrides)
    assert done.returncode == 0, done.stderr
    return [doc for doc in yaml.safe_load_all(done.stdout) if doc]


def refused(umbrella: Path, **overrides: str) -> str:
    """The stderr of a render that must be refused."""
    done = render(umbrella, overrides)
    assert done.returncode != 0, f"render should have failed, got:\n{done.stdout}"
    return done.stderr


def config_secret(docs: list[dict]) -> dict[str, str]:
    (secret,) = [doc for doc in docs if doc.get("kind") == "Secret" and doc["metadata"]["name"] == "insight-v3-core-config"]
    return secret["stringData"]


def test_a_lowercase_destination_renders_into_the_config_secret(umbrella: Path) -> None:
    data = config_secret(rendered(umbrella, **ALERTS_ON, **destination("ops-team")))

    assert data[f"{DESTINATIONS_PREFIX}ops-team__provider"] == "discord"
    assert data[f"{DESTINATIONS_PREFIX}ops-team__webhook_url"] == WEBHOOK


def test_a_destination_with_an_uppercase_letter_refuses_to_render(umbrella: Path) -> None:
    stderr = refused(umbrella, **ALERTS_ON, **destination("Ops-Team"))

    assert "insightV3Core.alerts.destinations.Ops-Team must be a lowercase name" in stderr


def test_a_destination_named_outside_the_service_charset_refuses_to_render(umbrella: Path) -> None:
    stderr = refused(umbrella, **ALERTS_ON, **destination("ops team"))

    assert "insightV3Core.alerts.destinations.ops team must be a lowercase name" in stderr


def test_destinations_in_values_under_gitops_refuse_to_render_without_a_sealed_secret(umbrella: Path) -> None:
    stderr = refused(umbrella, **ALERTS_ON, **GITOPS, **destination("ops"))

    assert "credentials.autoGenerate is false" in stderr


def test_a_sealed_secret_carries_the_destinations_under_gitops(umbrella: Path) -> None:
    docs = rendered(umbrella, **ALERTS_ON, **GITOPS, **{"insightV3Core.alerts.existingSecret": "sealed-alerts"})

    (deployment,) = [doc for doc in docs if doc.get("kind") == "Deployment" and doc["metadata"]["name"].endswith("-v3-core")]
    sources = deployment["spec"]["template"]["spec"]["containers"][0]["envFrom"]
    assert {"secretRef": {"name": "sealed-alerts"}} in sources


def test_alerts_off_renders_no_alerts_leaf_anywhere(umbrella: Path) -> None:
    text = yaml.safe_dump_all(rendered(umbrella))

    assert "__alerts__" not in text
