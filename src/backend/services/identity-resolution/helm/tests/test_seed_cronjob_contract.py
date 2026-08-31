"""Helm render-contract for the persons-seed CronJob.

The original bug was the ABSENCE of scheduling (#1690) — the seed existed but
nothing ever ran it — so the schedule wiring itself is contract, not
plumbing: these tests render the chart(s) with `helm template` and assert the
manifests the cluster would actually get. No cluster involved; runs anywhere
helm + PyYAML exist (CI: .github/workflows/identity-resolution-helm.yml).

The chart ships ONE CronJob: the seed, which publishes its refreshed log to
ClickHouse as its own final step. CronJobs are selected BY NAME, never as
"the sole CronJob in the render", so the suite does not break when another
job appears.

Covered: the schedule and exact args against the mounted gears config
(tolerance flags present, never `--force`); the SAME Secret/ConfigMap pair
the deployment uses; `seed.tenantDefaultId` env-overriding the Secret; `enabled=false`
removing that CronJob and nothing else; pod labels never matching the
Service selector (a pod that listens on nothing must not enter the
endpoints); and that no persons-sync CronJob is rendered.

Umbrella: the seed tenant render guard, the CronJob rendering when a tenant
is configured, and the gear-config knobs an operator sets at the
conventional `identityResolution.<key>` path reaching the gear that reads
them.
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

# name suffix -> (schedule, args) — the per-job contract facts. One entry
# today; the table stays so a second scheduled job arrives with its contract
# already asserted. The tolerance flags are deliberate: the schedule is a
# freshness backstop, and a backstop that is red on healthy stands (empty
# identity_inputs, a raced lock) trains everyone to ignore it — refusals
# surface as failed operations-journal rows instead.
JOBS = {"seed": ("*/15 * * * *", ["seed", "--busy-ok", "--guard-ok"])}

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
    "--set",
    "authenticator.oidc.sourceType=ms-entra",
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


def _cronjobs(docs: list[dict]) -> dict[str, dict]:
    """All CronJobs in the render, keyed by metadata name."""
    return {d["metadata"]["name"]: d for d in docs if d.get("kind") == "CronJob"}


def _cronjob(docs: list[dict], job: str) -> dict:
    """The seed/sync CronJob selected BY NAME — never 'the sole CronJob'.

    Matched on the identity-resolution fullname + the job suffix rather than
    an exact literal, so the same helper works for subchart renders
    (`contract-test-identity-resolution-<job>`) and umbrella renders (whose
    release/alias prefix differs).
    """
    matches = {
        name: doc for name, doc in _cronjobs(docs).items() if "identity-resolution" in name and name.endswith(f"-{job}")
    }
    assert len(matches) == 1, (
        f"expected exactly one identity-resolution {job} CronJob; present: {sorted(_cronjobs(docs))}"
    )
    return next(iter(matches.values()))


def _subchart_docs(*extra: str) -> list[dict]:
    rc, out, err = _render(SUBCHART, *SUBCHART_BASE, *extra)
    assert rc == 0, err
    return _docs(out)


@pytest.fixture(scope="module")
def default_docs() -> list[dict]:
    return _subchart_docs()


def _umbrella_gear_config(manifests: str) -> dict:
    """The identity gear's config out of an umbrella render.

    Selected by name: the git-cli-proxy subchart also mounts a gears file called
    `insight.yaml`, so "the ConfigMap carrying insight.yaml" is two documents.
    """
    named = [
        d
        for d in _docs(manifests)
        if d.get("kind") == "ConfigMap" and d["metadata"]["name"].endswith("-identity-resolution-gears-config")
    ]
    assert len(named) == 1, f"expected one identity gears ConfigMap, got {len(named)}"

    host = yaml.safe_load(named[0]["data"]["insight.yaml"])
    return host["gears"]["identity-resolution"]["config"]


def _job_container(cronjob: dict) -> dict:
    pod = cronjob["spec"]["jobTemplate"]["spec"]["template"]["spec"]
    assert pod["restartPolicy"] == "Never", pod
    (container,) = pod["containers"]
    return container


def test_default_render_ships_exactly_the_documented_cronjobs(default_docs) -> None:
    names = sorted(_cronjobs(default_docs))
    assert len(names) == len(JOBS), names
    for job in JOBS:
        _cronjob(default_docs, job)


def test_no_persons_sync_cronjob_is_scheduled(default_docs) -> None:
    """A scheduled sync would put the log and its ClickHouse snapshot back on
    two independent clocks — the ordering hazard the seed's own publish step
    removes. Nothing else in this suite would notice it returning."""
    assert not [n for n in _cronjobs(default_docs) if n.endswith("-sync")], sorted(_cronjobs(default_docs))


@pytest.mark.parametrize("job", JOBS)
def test_cronjob_exists_by_default_with_documented_schedule(default_docs, job: str) -> None:
    cj = _cronjob(default_docs, job)
    assert cj["spec"]["schedule"] == JOBS[job][0]
    assert cj["spec"]["concurrencyPolicy"] == "Forbid"


@pytest.mark.parametrize("job", JOBS)
def test_cronjob_runs_its_subcommand_against_the_mounted_config(default_docs, job: str) -> None:
    container = _job_container(_cronjob(default_docs, job))
    assert container["command"] == ["/app/identity-resolution"]
    assert container["args"] == ["-c", "/app/config/insight.yaml", *JOBS[job][1]]
    # A CronJob must never run forced — --force overrides the input guards and
    # is a deliberate manual act. Tolerating a refusal (--guard-ok) is not
    # forcing: the run publishes nothing and journals why.
    assert "--force" not in container["args"]


@pytest.mark.parametrize("job", JOBS)
def test_cronjob_uses_the_deployments_secret_and_configmap(default_docs, job: str) -> None:
    cj = _cronjob(default_docs, job)
    deploy = _the(default_docs, "Deployment")
    container = _job_container(cj)

    secret_refs = [e["secretRef"]["name"] for e in container["envFrom"] if "secretRef" in e]
    deploy_secret_refs = [
        e["secretRef"]["name"]
        for e in deploy["spec"]["template"]["spec"]["containers"][0]["envFrom"]
        if "secretRef" in e
    ]
    assert secret_refs == deploy_secret_refs == ["test-secret"]

    cj_volumes = cj["spec"]["jobTemplate"]["spec"]["template"]["spec"]["volumes"]
    deploy_volumes = deploy["spec"]["template"]["spec"]["volumes"]
    cj_cm = next(v["configMap"]["name"] for v in cj_volumes if "configMap" in v)
    deploy_cm = next(v["configMap"]["name"] for v in deploy_volumes if "configMap" in v)
    assert cj_cm == deploy_cm


@pytest.mark.parametrize("job", JOBS)
def test_tenant_value_overrides_the_secret_via_env(default_docs, job: str) -> None:
    # Default: no explicit env — the Secret is the tenant source.
    container = _job_container(_cronjob(default_docs, job))
    assert "env" not in container, container.get("env")

    docs = _subchart_docs("--set", f"{job}.tenantDefaultId={TENANT}")
    container = _job_container(_cronjob(docs, job))
    env = {e["name"]: e["value"] for e in container["env"]}
    assert env == {"APP__gears__identity_resolution__config__tenant_default_id": TENANT}


@pytest.mark.parametrize("job", JOBS)
def test_disabling_one_job_removes_only_that_cronjob(job: str) -> None:
    docs = _subchart_docs("--set", f"{job}.enabled=false")
    jobs = _cronjobs(docs)
    assert f"contract-test-identity-resolution-{job}" not in jobs, sorted(jobs)
    # Every other CronJob and the rest of the chart are untouched.
    for other in (j for j in JOBS if j != job):
        _cronjob(docs, other)
    _the(docs, "Deployment")
    _the(docs, "Service")


@pytest.mark.parametrize("job", JOBS)
def test_job_pod_labels_never_match_the_service_selector(default_docs, job: str) -> None:
    """A seed/sync pod listens on nothing: if the Service selector matched
    it, it would enter the endpoints and blackhole live traffic during every
    run."""
    selector = _the(default_docs, "Service")["spec"]["selector"]
    pod_labels = _cronjob(default_docs, job)["spec"]["jobTemplate"]["spec"]["template"]["metadata"]["labels"]
    assert any(pod_labels.get(k) != v for k, v in selector.items()), (
        f"{job} pod labels {pod_labels} satisfy the Service selector {selector}"
    )


# ── umbrella: the tenant render guard ─────────────────────────────────────


# `umbrella_deps` is session-scoped in conftest.py — one vendor per run.


def test_umbrella_refuses_enabled_seed_without_a_tenant(umbrella_deps) -> None:
    rc, _, err = _render(umbrella_deps, *UMBRELLA_BASE)
    assert rc != 0
    assert "requires a tenant" in err, err


def test_umbrella_renders_the_cronjobs_with_a_tenant(umbrella_deps) -> None:
    rc, out, err = _render(umbrella_deps, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert rc == 0, err
    docs = _docs(out)
    for job in JOBS:
        _cronjob(docs, job)


def test_umbrella_accepts_the_explicit_seed_tenant_alone(umbrella_deps) -> None:
    rc, out, err = _render(umbrella_deps, *UMBRELLA_BASE, "--set", f"identityResolution.seed.tenantDefaultId={TENANT}")
    assert rc == 0, err
    container = _job_container(_cronjob(_docs(out), "seed"))
    env = {e["name"]: e["value"] for e in container.get("env", [])}
    assert env.get("APP__gears__identity_resolution__config__tenant_default_id") == TENANT


def test_umbrella_disabled_seed_needs_no_tenant(umbrella_deps) -> None:
    rc, out, err = _render(umbrella_deps, *UMBRELLA_BASE, "--set", "identityResolution.seed.enabled=false")
    assert rc == 0, err
    jobs = _cronjobs(_docs(out))
    assert not any("identity-resolution" in n and n.endswith("-seed") for n in jobs), sorted(jobs)


def test_umbrella_carries_the_roster_source_to_the_gear_that_reads_it(umbrella_deps) -> None:
    """An operator sets this at `identityResolution.rosterSourceType`.

    Nothing else proves that path: the subchart tests set the key directly, so a
    value the umbrella dropped would leave the seed minting nothing at all — with
    a successful run and an empty counter as the only symptom.
    """
    rc, out, err = _render(
        umbrella_deps,
        *UMBRELLA_BASE,
        "--set",
        f"global.tenantDefaultId={TENANT}",
        "--set",
        "identityResolution.rosterSourceType=bamboohr",
    )
    assert rc == 0, err

    assert _umbrella_gear_config(out)["roster_source_type"] == "bamboohr"


def test_umbrella_names_no_roster_by_default(umbrella_deps) -> None:
    rc, out, err = _render(umbrella_deps, *UMBRELLA_BASE, "--set", f"global.tenantDefaultId={TENANT}")
    assert rc == 0, err

    assert _umbrella_gear_config(out)["roster_source_type"] == "", "an upgrade must not start minting persons by itself"
