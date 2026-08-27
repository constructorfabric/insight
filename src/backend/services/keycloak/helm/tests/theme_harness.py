"""Render harness shared by the Helm contract tests in this directory.

Rendered from a SYNTHETIC values set, never from a
`deploy/gitops/environments/*` overlay: an overlay is free to change what it
enables, and a contract test that follows it stops asserting silently instead
of failing.

Named `theme_harness` rather than living in `conftest.py` because pytest
imports a bare `conftest` by module name — a second helm suite with its own
`conftest.py` (identity-resolution) then collides on any invocation that
collects both.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import yaml

HERE = Path(__file__).resolve()
SUBCHART = HERE.parents[1]  # .../keycloak/helm
REPO_ROOT = HERE.parents[6]
UMBRELLA = REPO_ROOT / "charts" / "insight"
CANONICAL_REALMS = sorted(
    (REPO_ROOT / "deploy" / "gitops" / "environments").glob(
        "*/keycloak/realms/insight-broker.yaml"
    )
)

RELEASE = "contract-test"

THEME_NAME = "insight"
THEME_SOURCE = SUBCHART / "theme" / THEME_NAME / "login" / "login-page-expired.ftl"
MOUNT_PATH = f"/opt/keycloak/themes/{THEME_NAME}/login"
THEME_ROOT = "/opt/keycloak/themes"

# Every value the subchart declares `required` — enough to render, nothing
# that the assertions then read back.
SUBCHART_BASE = [
    "--set",
    "admin.existingSecret=kc-admin",
    "--set",
    "database.host=mariadb",
    "--set",
    "database.username=keycloak",
    "--set",
    "database.passwordSecret.name=kc-db",
    "--set",
    "database.passwordSecret.key=password",
    "--set",
    "hostname=https://example.test/kc",
]

# The umbrella's own required wiring (infra dials, authenticator OIDC) plus
# the Keycloak block — the minimum that renders, nothing the assertions read.
UMBRELLA_BASE = [
    "--set",
    "clickhouse.host=ch",
    "--set",
    "clickhouse.username=u",
    "--set",
    "clickhouse.password=p",
    "--set",
    "clickhouse.database=insight",
    "--set",
    "mariadb.host=m",
    "--set",
    "mariadb.username=insight",
    "--set",
    "mariadb.password=pw",
    "--set",
    "mariadb.database=insight",
    "--set",
    "redis.host=redis",
    "--set",
    "redis.password=rp",
    "--set",
    "redpanda.brokers=rp:9092",
    "--set",
    "ingestion.reconcile.tenantId=default",
    "--set",
    "global.tenantDefaultId=3e1d5a65-434c-95b4-8c1b-eb8f53a39bab",
    "--set",
    "authenticator.oidc.issuerUrl=https://idp",
    "--set",
    "authenticator.oidc.clientId=c",
    "--set",
    "authenticator.oidc.clientSecret=s",
    "--set",
    "authenticator.oidc.redirectUri=https://x/cb",
    "--set",
    "authenticator.oidc.sourceType=ms-entra",
    "--set",
    "keycloak.deploy=true",
    "--set",
    "keycloak.admin.existingSecret=kc-admin",
    "--set",
    "keycloak.database.host=mariadb",
    "--set",
    "keycloak.database.username=keycloak",
    "--set",
    "keycloak.database.passwordSecret.name=kc-db",
    "--set",
    "keycloak.database.passwordSecret.key=password",
    "--set",
    "keycloak.hostname=https://example.test/kc",
]


def render(chart: Path, *extra: str) -> list[dict]:
    """`helm template` a chart into its parsed documents."""
    proc = subprocess.run(
        ["helm", "template", RELEASE, str(chart), *extra],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    return [doc for doc in yaml.safe_load_all(proc.stdout) if doc]


def render_error(chart: Path, *extra: str) -> str:
    """`helm template` a chart that must NOT render; returns the message."""
    proc = subprocess.run(
        ["helm", "template", RELEASE, str(chart), *extra],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert proc.returncode != 0, f"expected a render failure, got:\n{proc.stdout[:400]}"
    return proc.stderr


def one(docs: list[dict], kind: str, name: str) -> dict:
    """The single `kind` named `<release>-<name>`."""
    want = f"{RELEASE}-{name}"
    found = [d for d in docs if d.get("kind") == kind and d["metadata"]["name"] == want]
    assert len(found) == 1, f"expected one {kind} named {want!r}, got {len(found)}"
    return found[0]


def theme_configmap(docs: list[dict]) -> dict:
    return one(docs, "ConfigMap", "keycloak-theme")


def keycloak_pod(docs: list[dict]) -> dict:
    return one(docs, "Deployment", "keycloak")["spec"]["template"]


def theme_properties(docs: list[dict]) -> dict[str, str]:
    raw = theme_configmap(docs)["data"]["theme.properties"]
    return dict(
        line.split("=", 1) for line in raw.splitlines() if line and "=" in line
    )


def assert_theme_payload(docs: list[dict]) -> None:
    """The theme is only useful if all three of its parts survive rendering."""
    data = theme_configmap(docs)["data"]
    ftl = data["login-page-expired.ftl"]

    # `.Files.Get` returns "" on a miss and the install still succeeds, so the
    # transport is pinned to the source rather than sampled for a substring.
    assert ftl.rstrip("\n") == THEME_SOURCE.read_text().rstrip("\n")

    # Without a parent the theme owns no other login template and every other
    # Keycloak-rendered page in the realm 500s. `keycloak` (LOGIN_V1) is
    # deprecated in KC 26 and would downgrade those pages.
    assert theme_properties(docs)["parent"] == "keycloak.v2"

    # The knob is only live while the template reads the key the chart writes.
    assert "properties.insightReturnUrl" in ftl

    # Three mechanisms, all load-bearing: no-JS fallback, the JS path that
    # keeps this page out of history, and the visible link the JS reads.
    assert 'http-equiv="refresh"' in ftl
    assert "window.location.replace" in ftl
    assert ftl.count("${returnUrl}") >= 2
