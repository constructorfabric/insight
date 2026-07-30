"""Helm render-contract for the persons-seed CronJob (#1690).

The original bug was the ABSENCE of scheduling — the seed existed but nothing
ever ran it — so the schedule wiring itself is contract, not plumbing: these
tests render the chart(s) with `helm template` and assert the manifests the
cluster would actually get. No cluster involved; runs anywhere helm + PyYAML
exist (CI: .github/workflows/identity-resolution-helm.yml).

Covered:
  * the CronJob exists by default with the documented schedule and the exact
    `seed` command/args against the mounted gears config;
  * config comes from the SAME Secret/ConfigMap pair the deployment uses;
  * `seed.tenantDefaultId` env-overrides the Secret (k8s `env` beats
    `envFrom`) — the standalone-install tenant source;
  * `seed.enabled=false` removes the CronJob and nothing else;
  * the seed pod labels do NOT match the Service selector (a pod that
    listens on nothing must never enter the Service's endpoints);
  * the umbrella refuses to render when the seed is enabled but no tenant is
    configured — only on the path where the umbrella composes the config
    Secret itself (`credentials.autoGenerate`).
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve()
SUBCHART = HERE.parents[1]  # .../identity-resolution/helm
REPO_ROOT = HERE.parents[6]
UMBRELLA = REPO_ROOT / "charts" / "insight"

TENANT = "3e1d5a65-434c-95b4-8c1b-eb8f53a39bab"

# Minimum viable subchart install (mirrors the umbrella's wiring).
SUBCHART_BASE = [
    "--set",
    "image.tag=0.0.0-test",
    "--set",
    "existingSecret=test-secret",
    "--set",
    "gateway.issuer=https://issuer.test",
]

# Minimum viable umbrella install on the credentials.autoGenerate path (the
# only path where the umbrella composes the identity-resolution config Secret
# and can therefore vouch for the tenant inside it).
UMBRELLA_BASE = [
    "--set",
    "identityResolution.deploy=true",
    "--set",
    "identityResolution.image.tag=0.0.0-test",
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
    "authenticator.oidc.issuerUrl=https://idp",
    "--set",
    "authenticator.oidc.clientId=c",
    "--set",
    "authenticator.oidc.clientSecret=s",
    "--set",
    "authenticator.oidc.redirectUri=https://x/cb",
]


def _render(chart: Path, *extra: str) -> tuple[int, str, str]:
    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        ["helm", "template", "contract-test", str(chart), *extra],  # noqa: S607
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    return proc.returncode, proc.stdout, proc.stderr


def _docs(manifests: str) -> list[dict]:
    return [d for d in yaml.safe_load_all(manifests) if isinstance(d, dict)]


def _the(docs: list[dict], kind: str) -> dict:
    matches = [d for d in docs if d.get("kind") == kind]
    assert len(matches) == 1, f"expected exactly one {kind}, got {len(matches)}"
    return matches[0]


def _subchart_docs(*extra: str) -> list[dict]:
    rc, out, err = _render(SUBCHART, *SUBCHART_BASE, *extra)
    assert rc == 0, err
    return _docs(out)


@pytest.fixture(scope="module")
def default_docs() -> list[dict]:
    return _subchart_docs()


def _seed_container(cronjob: dict) -> dict:
    pod = cronjob["spec"]["jobTemplate"]["spec"]["template"]["spec"]
    assert pod["restartPolicy"] == "Never", pod
    (container,) = pod["containers"]
    return container


def test_cronjob_exists_by_default_with_documented_schedule(default_docs) -> None:
    cj = _the(default_docs, "CronJob")
    assert cj["metadata"]["name"] == "contract-test-identity-resolution-seed"
    assert cj["spec"]["schedule"] == "30 6 * * *"
    assert cj["spec"]["concurrencyPolicy"] == "Forbid"


def test_cronjob_runs_the_seed_subcommand_against_the_mounted_config(default_docs) -> None:
    container = _seed_container(_the(default_docs, "CronJob"))
    assert container["command"] == ["/app/identity-resolution"]
    assert container["args"] == ["-c", "/app/config/insight.yaml", "seed"]
    # The CronJob must never run forced — --force is a deliberate manual act.
    assert "--force" not in container["args"]


def test_cronjob_uses_the_deployments_secret_and_configmap(default_docs) -> None:
    cj = _the(default_docs, "CronJob")
    deploy = _the(default_docs, "Deployment")
    container = _seed_container(cj)

    secret_refs = [e["secretRef"]["name"] for e in container["envFrom"]]
    deploy_secret_refs = [
        e["secretRef"]["name"] for e in deploy["spec"]["template"]["spec"]["containers"][0]["envFrom"]
    ]
    assert secret_refs == deploy_secret_refs == ["test-secret"]

    cj_volumes = cj["spec"]["jobTemplate"]["spec"]["template"]["spec"]["volumes"]
    deploy_volumes = deploy["spec"]["template"]["spec"]["volumes"]
    cj_cm = next(v["configMap"]["name"] for v in cj_volumes if "configMap" in v)
    deploy_cm = next(v["configMap"]["name"] for v in deploy_volumes if "configMap" in v)
    assert cj_cm == deploy_cm


def test_tenant_value_overrides_the_secret_via_env(default_docs) -> None:
    # Default: no explicit env — the Secret is the tenant source.
    container = _seed_container(_the(default_docs, "CronJob"))
    assert "env" not in container, container.get("env")

    docs = _subchart_docs("--set", f"seed.tenantDefaultId={TENANT}")
    container = _seed_container(_the(docs, "CronJob"))
    env = {e["name"]: e["value"] for e in container["env"]}
    assert env == {"APP__gears__identity-resolution__config__tenant_default_id": TENANT}


def test_seed_disabled_removes_only_the_cronjob() -> None:
    docs = _subchart_docs("--set", "seed.enabled=false")
    assert not [d for d in docs if d.get("kind") == "CronJob"]
    # The rest of the chart is untouched.
    _the(docs, "Deployment")
    _the(docs, "Service")


def test_seed_pod_labels_never_match_the_service_selector(default_docs) -> None:
    """A seed pod listens on nothing: if the Service selector matched it, it
    would enter the endpoints and blackhole live traffic during every run."""
    selector = _the(default_docs, "Service")["spec"]["selector"]
    pod_labels = _the(default_docs, "CronJob")["spec"]["jobTemplate"]["spec"]["template"][
        "metadata"
    ]["labels"]
    assert any(pod_labels.get(k) != v for k, v in selector.items()), (
        f"seed pod labels {pod_labels} satisfy the Service selector {selector}"
    )


# ── umbrella: the tenant render guard ─────────────────────────────────────


@pytest.fixture(scope="module")
def umbrella_deps() -> Path:
    subprocess.run(  # noqa: S603, S607 — refresh the vendored subcharts
        ["helm", "dependency", "update", str(UMBRELLA)],
        capture_output=True,
        text=True,
        timeout=300,
        check=True,
    )
    return UMBRELLA


def test_umbrella_refuses_enabled_seed_without_a_tenant(umbrella_deps) -> None:
    rc, _, err = _render(umbrella_deps, *UMBRELLA_BASE)
    assert rc != 0
    assert "requires a tenant" in err, err


def test_umbrella_renders_the_cronjob_with_a_tenant(umbrella_deps) -> None:
    rc, out, err = _render(
        umbrella_deps, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}"
    )
    assert rc == 0, err
    _the(_docs(out), "CronJob")


def test_umbrella_accepts_the_explicit_seed_tenant_alone(umbrella_deps) -> None:
    rc, out, err = _render(
        umbrella_deps, *UMBRELLA_BASE, "--set", f"identityResolution.seed.tenantDefaultId={TENANT}"
    )
    assert rc == 0, err
    container = _seed_container(_the(_docs(out), "CronJob"))
    env = {e["name"]: e["value"] for e in container.get("env", [])}
    assert env.get("APP__gears__identity-resolution__config__tenant_default_id") == TENANT


def test_umbrella_disabled_seed_needs_no_tenant(umbrella_deps) -> None:
    rc, out, err = _render(
        umbrella_deps, *UMBRELLA_BASE, "--set", "identityResolution.seed.enabled=false"
    )
    assert rc == 0, err
    assert not [d for d in _docs(out) if d.get("kind") == "CronJob"]
