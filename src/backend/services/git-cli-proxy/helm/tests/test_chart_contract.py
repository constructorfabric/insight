"""Helm render-contract for the git-cli-proxy chart.

This is the first chart in the repo that carries a PersistentVolumeClaim, and
its correctness rests on facts that are invisible in a diff: the cache volume
admits exactly one POD — ReadWriteOnce would admit many on one node — because
the process holds its locks in memory; the service is Airbyte-only, so its ingress must be closed by
default; and the byte budgets must render as integers, not as the scientific
notation Helm produces for large YAML numbers (the service parses them as
u64 and would refuse to boot).

No cluster involved; runs anywhere helm + PyYAML exist.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]
RELEASE = "insight"


def render(**overrides: str) -> list[dict]:
    args = ["helm", "template", RELEASE, str(CHART), "--set", "existingSecret=cfg"]
    for key, value in overrides.items():
        args += ["--set", f"{key.replace('__', '.')}={value}"]
    out = subprocess.run(args, capture_output=True, text=True, check=True).stdout
    return [doc for doc in yaml.safe_load_all(out) if doc]


def of_kind(docs: list[dict], kind: str) -> list[dict]:
    return [doc for doc in docs if doc.get("kind") == kind]


def one(docs: list[dict], kind: str) -> dict:
    found = of_kind(docs, kind)
    assert len(found) == 1, f"expected exactly one {kind}, got {len(found)}"
    return found[0]


def gear_config(docs: list[dict]) -> dict:
    """The git-cli-proxy gear's own config block, parsed out of the ConfigMap."""
    for doc in of_kind(docs, "ConfigMap"):
        raw = doc.get("data", {}).get("insight.yaml")
        if raw is None:
            continue
        return yaml.safe_load(raw)["gears"]["git-cli-proxy"]["config"]
    raise AssertionError("no ConfigMap carries insight.yaml")


def test_rendered_config_has_the_types_the_service_parses():
    """The chart's output must deserialize, not merely be valid YAML.

    An empty `range` leaves its key with no value — YAML null — and the
    service refuses to start on a null where it expects a list. Nothing else
    in CI parses the chart's own output, so this is the only gate that sees it.
    """
    config = gear_config(render())

    assert isinstance(config["allowed_repo_hosts"], list), (
        "an empty allowlist must render as [], not null"
    )
    assert isinstance(config["data_dir"], str) and config["data_dir"]
    for numeric in (
        "disk_budget_bytes",
        "max_repo_bytes",
        "default_max_staleness_seconds",
        "heavy_ops_concurrency",
    ):
        assert isinstance(config[numeric], int), f"{numeric} must render as an integer"
    assert config["allow_file_repos"] is False


def test_a_named_allowlist_host_reaches_the_config():
    config = gear_config(render(cache__allowedRepoHosts="{gitlab.example}"))
    assert config["allowed_repo_hosts"] == ["gitlab.example"]


def test_the_pod_receives_the_platform_emission_contract():
    """§4.3's metrics are exported only if the OTEL_* env vars reach the pod.

    They live in the umbrella's platform ConfigMap. A deployment that wires
    only its own Secret computes every metric and exports none of them, and
    nothing else in CI notices — the instruments still register.
    """
    deployment = one(render(), "Deployment")
    sources = deployment["spec"]["template"]["spec"]["containers"][0]["envFrom"]
    refs = [s["configMapRef"]["name"] for s in sources if "configMapRef" in s]
    assert any(name.endswith("-platform") for name in refs), (
        f"the platform ConfigMap must be wired in, got {sources}"
    )


def test_cache_volume_is_single_writer():
    """One writer, enforced in three places that are one decision."""
    docs = render()
    pvc = one(docs, "PersistentVolumeClaim")
    # ReadWriteOnce admits many pods on ONE node, which is not what the
    # process-local locking needs; ReadWriteOncePod is the accurate promise.
    assert pvc["spec"]["accessModes"] == ["ReadWriteOncePod"]

    deployment = one(docs, "Deployment")
    assert deployment["spec"]["replicas"] == 1, "in-memory locks admit one writer"
    assert deployment["spec"]["strategy"]["type"] == "Recreate", (
        "a rolling update would have two pods racing for one volume"
    )


def test_more_than_one_replica_refuses_to_render():
    """A second replica does not share the cache; it races it."""
    with pytest.raises(subprocess.CalledProcessError) as raised:
        render(replicaCount="2")
    assert "replicaCount must be 1" in raised.value.stderr


def test_cache_survives_uninstall():
    pvc = one(render(), "PersistentVolumeClaim")
    assert pvc["metadata"]["annotations"]["helm.sh/resource-policy"] == "keep", (
        "dropping the cache triggers a re-clone storm; that is an operator call"
    )


def test_byte_budgets_render_as_integers():
    """Helm parses YAML numbers as float64: a large budget would otherwise
    render as 4.6e+10 and the service would fail to deserialize it."""
    config = one(render(), "ConfigMap")["data"]["insight.yaml"]
    parsed = yaml.safe_load(config)["gears"]["git-cli-proxy"]["config"]

    for field in ("disk_budget_bytes", "max_repo_bytes", "default_max_staleness_seconds", "heavy_ops_concurrency"):
        assert isinstance(parsed[field], int), f"{field} must be an integer"
    assert parsed["disk_budget_bytes"] > parsed["max_repo_bytes"]


def test_token_never_reaches_the_configmap():
    docs = render()
    config = one(docs, "ConfigMap")["data"]["insight.yaml"]
    assert yaml.safe_load(config)["gears"]["git-cli-proxy"]["config"]["proxy_token"] == ""

    container = one(docs, "Deployment")["spec"]["template"]["spec"]["containers"][0]
    secrets = [
        ref["secretRef"]["name"] for ref in container["envFrom"] if "secretRef" in ref
    ]
    assert secrets == ["cfg"], "the token arrives from the Secret, nowhere else"
    # Other envFrom sources are allowed (the platform ConfigMap carries the
    # emission contract) but must never be a second source of the token.
    assert not any(
        "token" in str(ref.get("configMapRef", {})).lower() for ref in container["envFrom"]
    )


def test_ingress_is_denied_until_a_namespace_is_allowed():
    policy = one(render(), "NetworkPolicy")
    assert policy["spec"]["policyTypes"] == ["Ingress"]
    assert not policy["spec"].get("ingress"), "an unconfigured allow-list must deny, not open, this API"


def test_allowed_namespaces_become_ingress_rules():
    docs = render(
        **{
            "networkPolicy__allowedNamespaceLabels[0]__key": "kubernetes.io/metadata.name",
            "networkPolicy__allowedNamespaceLabels[0]__values[0]": "airbyte",
        }
    )
    rules = one(docs, "NetworkPolicy")["spec"]["ingress"]
    assert len(rules) == 1
    selector = rules[0]["from"][0]["namespaceSelector"]["matchLabels"]
    assert selector == {"kubernetes.io/metadata.name": "airbyte"}


def test_no_ingress_object_is_ever_rendered():
    """The service is Airbyte-only: it must never be reachable from outside
    the cluster, and no gateway route points at it."""
    assert of_kind(render(), "Ingress") == []


def test_writable_paths_are_explicit_under_a_read_only_root():
    container = one(render(), "Deployment")["spec"]["template"]["spec"]["containers"][0]
    assert container["securityContext"]["readOnlyRootFilesystem"] is True

    mounts = {mount["mountPath"] for mount in container["volumeMounts"]}
    assert {"/app/data/repos", "/app/data/home", "/tmp"} <= mounts, (
        "the cache, the gears home dir and the grpc UDS are the writable paths"
    )


def test_cache_mount_path_matches_the_configured_data_dir():
    docs = render()
    data_dir = yaml.safe_load(one(docs, "ConfigMap")["data"]["insight.yaml"])["gears"]["git-cli-proxy"]["config"][
        "data_dir"
    ]

    container = one(docs, "Deployment")["spec"]["template"]["spec"]["containers"][0]
    cache_mount = next(mount for mount in container["volumeMounts"] if mount["name"] == "cache")
    assert cache_mount["mountPath"] == data_dir, (
        "a config pointing outside the volume would fill the container's rootfs"
    )


def test_probes_target_the_public_health_path():
    container = one(render(), "Deployment")["spec"]["template"]["spec"]["containers"][0]
    for probe in ("livenessProbe", "readinessProbe"):
        assert container[probe]["httpGet"]["path"] == "/healthz", f"{probe} must not require the bearer token"


def test_missing_secret_fails_the_render():
    with pytest.raises(subprocess.CalledProcessError) as excinfo:
        subprocess.run(["helm", "template", RELEASE, str(CHART)], capture_output=True, text=True, check=True)
    assert "existingSecret is required" in excinfo.value.stderr


def render_fails(**overrides: str) -> str:
    """The stderr of a render that must be refused."""
    args = ["helm", "template", RELEASE, str(CHART), "--set", "existingSecret=cfg"]
    for key, value in overrides.items():
        args += ["--set", f"{key.replace('__', '.')}={value}"]
    done = subprocess.run(args, capture_output=True, text=True, check=False)
    assert done.returncode != 0, f"render should have failed, got:\n{done.stdout}"
    return done.stderr


@pytest.mark.parametrize(
    ("overrides", "expected"),
    [
        # The volume filling up is enforced by kubelet as a pod eviction, so
        # the app budget has to leave headroom rather than track the volume.
        ({"cache__diskBudgetBytes": "49000000000"}, "at most 90%"),
        ({"cache__diskBudgetBytes": "20000000000"}, "under 50%"),
        # A repository admitted at the per-repo cap must fit in the cache.
        ({"cache__maxRepoBytes": "99000000000"}, "exceeds cache.diskBudgetBytes"),
        # An unparseable size would make the guard vacuous, so it is refused
        # rather than read as zero.
        ({"persistence__size": "50Gigs"}, "unsupported unit"),
        ({"persistence__size": "big"}, "not a Kubernetes quantity"),
    ],
)
def test_a_budget_that_cannot_fit_the_volume_is_refused(overrides, expected):
    assert expected in render_fails(**overrides)


def test_the_shipped_budget_fits_the_shipped_volume():
    """The committed defaults must satisfy the guard they ship with."""
    claim = one(render(), "PersistentVolumeClaim")
    assert claim["spec"]["resources"]["requests"]["storage"] == "50Gi"


def test_the_cache_volume_follows_the_global_storage_class():
    """The umbrella documents global.storageClass as visible to every
    subchart. The PVC is resource-policy: keep, so a wrong class costs a
    manual delete."""
    assert "storageClassName" not in one(render(), "PersistentVolumeClaim")["spec"], (
        "unset must mean the cluster default, not an empty string"
    )

    from_global = one(render(global__storageClass="fast-ssd"), "PersistentVolumeClaim")
    assert from_global["spec"]["storageClassName"] == "fast-ssd"

    explicit = one(
        render(global__storageClass="fast-ssd", persistence__storageClass="local-path"),
        "PersistentVolumeClaim",
    )
    assert explicit["spec"]["storageClassName"] == "local-path", (
        "an explicit subchart value must win over the global default"
    )
