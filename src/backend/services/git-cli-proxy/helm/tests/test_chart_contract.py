"""Helm render-contract for the git-cli-proxy chart.

This is the first chart in the repo that carries a PersistentVolumeClaim, and
its correctness rests on facts that are invisible in a diff: the cache volume
is ReadWriteOnce and the process holds its locks in memory, so exactly one pod
may ever bind it; the service is Airbyte-only, so its ingress must be closed by
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


def test_cache_volume_is_single_writer():
    """RWO + Recreate + one replica are one decision, not three."""
    docs = render()
    pvc = one(docs, "PersistentVolumeClaim")
    assert pvc["spec"]["accessModes"] == ["ReadWriteOnce"]

    deployment = one(docs, "Deployment")
    assert deployment["spec"]["replicas"] == 1, "in-memory locks admit one writer"
    assert deployment["spec"]["strategy"]["type"] == "Recreate", (
        "a rolling update would have two pods racing for one RWO volume"
    )


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
    sources = [ref["secretRef"]["name"] for ref in container["envFrom"]]
    assert sources == ["cfg"], "the token arrives from the Secret, nowhere else"


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
