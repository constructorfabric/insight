"""The subchart's own contract: the experiment namespace has ONE source, the
RBAC stays confined to the experiment trio, and the deploy shape (single
replica, dedicated ServiceAccount) cannot silently widen."""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

HERE = Path(__file__).resolve()
SUBCHART = HERE.parents[1]

SUBCHART_BASE = ["--set", "image.tag=0.0.0-test", "--set", "gateway.issuer=https://issuer.test"]


def _render(*extra: str) -> tuple[int, str, str]:
    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        ["helm", "template", "contract-test", str(SUBCHART), *SUBCHART_BASE, *extra],  # noqa: S607
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    return proc.returncode, proc.stdout, proc.stderr


def _docs(*extra: str) -> list[dict]:
    code, out, err = _render(*extra)
    assert code == 0, f"render failed: {err}"
    return [d for d in yaml.safe_load_all(out) if isinstance(d, dict)]


def _kind(docs: list[dict], kind: str) -> list[dict]:
    return [d for d in docs if d.get("kind") == kind]


def _gear_config(docs: list[dict]) -> dict:
    configmaps = _kind(docs, "ConfigMap")
    assert len(configmaps) == 1, f"expected one ConfigMap, got {len(configmaps)}"
    host = yaml.safe_load(configmaps[0]["data"]["insight.yaml"])
    return host["gears"]["previews"]["config"]


def test_the_experiments_namespace_has_one_source() -> None:
    """`experiments.namespace` must reach the gear config, the Namespace
    object and the Role/RoleBinding together — set independently they drift
    and every create 403s."""
    docs = _docs("--set", "experiments.namespace=custom-previews-ns")

    assert _gear_config(docs)["namespace"] == "custom-previews-ns"
    assert _kind(docs, "Namespace")[0]["metadata"]["name"] == "custom-previews-ns"
    assert _kind(docs, "Role")[0]["metadata"]["namespace"] == "custom-previews-ns"
    assert _kind(docs, "RoleBinding")[0]["metadata"]["namespace"] == "custom-previews-ns"


def test_rbac_grants_exactly_the_experiment_trio() -> None:
    """create/list/delete on deployments, services and httproutes — nothing
    else, and only via a namespaced Role (no ClusterRole may appear)."""
    docs = _docs()

    assert not _kind(docs, "ClusterRole")
    assert not _kind(docs, "ClusterRoleBinding")

    rules = _kind(docs, "Role")[0]["rules"]
    granted = {(group, resource) for rule in rules for group in rule["apiGroups"] for resource in rule["resources"]}
    assert granted == {("apps", "deployments"), ("", "services"), ("gateway.networking.k8s.io", "httproutes")}
    for rule in rules:
        assert sorted(rule["verbs"]) == ["create", "delete", "list"], f"should stay create/list/delete: {rule}"


def test_the_rolebinding_targets_the_pods_serviceaccount() -> None:
    docs = _docs("--namespace", "insight")

    account = _kind(docs, "ServiceAccount")[0]["metadata"]["name"]
    deployment = _kind(docs, "Deployment")[0]
    assert deployment["spec"]["template"]["spec"]["serviceAccountName"] == account

    (subject,) = _kind(docs, "RoleBinding")[0]["subjects"]
    assert subject == {"kind": "ServiceAccount", "name": account, "namespace": "insight"}


def test_an_unmanaged_namespace_still_gets_the_rbac() -> None:
    docs = _docs("--set", "experiments.manageNamespace=false")

    assert not _kind(docs, "Namespace")
    assert len(_kind(docs, "Role")) == 1
    assert len(_kind(docs, "RoleBinding")) == 1


@pytest.mark.parametrize("replicas", [0, 2])
def test_anything_but_one_replica_is_refused(replicas: int) -> None:
    """The TTL sweep has no leader election and zero pods serves nothing —
    the chart, not a reviewer, holds the exactly-one-replica line."""
    code, _, err = _render("--set", f"replicaCount={replicas}")

    assert code != 0, f"should refuse replicaCount={replicas}"
    assert "leader election" in err


def test_route_host_defaults_empty_so_creates_fail_closed() -> None:
    assert _gear_config(_docs())["route_host"] == ""


def test_the_registry_token_rides_a_secret_env_and_never_the_configmap() -> None:
    docs = _docs("--set", "experiments.registryTokenSecret=registry-read")

    config = _gear_config(docs)
    assert config["registry_url"] == "https://ghcr.io"
    assert "registry_token" not in config

    container = _kind(docs, "Deployment")[0]["spec"]["template"]["spec"]["containers"][0]
    (env,) = container["env"]
    assert env["name"] == "APP__gears__previews__config__registry_token"
    assert env["valueFrom"]["secretKeyRef"] == {"name": "registry-read", "key": "token"}


def test_without_a_registry_secret_no_env_is_rendered() -> None:
    container = _kind(_docs(), "Deployment")[0]["spec"]["template"]["spec"]["containers"][0]

    assert "env" not in container


def test_experiment_knobs_reach_the_gear_config() -> None:
    config = _gear_config(
        _docs(
            "--set",
            "experiments.routeHost=preview.example.com",
            "--set",
            "experiments.maxExperiments=3",
            "--set",
            "sharedGateway.sectionName=https",
        )
    )

    assert config["route_host"] == "preview.example.com"
    assert config["max_experiments"] == 3
    assert config["gateway_section_name"] == "https"
    assert config["gateway_name"] == "insight"
    assert config["gateway_namespace"] == "insight-infra"
    assert config["base_path"] == "/exp"
    assert config["default_ttl_days"] == 7
    assert config["max_ttl_days"] == 30
    assert config["sweep_interval_secs"] == 300


@pytest.mark.parametrize(
    ("key", "expected"), [("auth_disabled", False), ("cors_enabled", False), ("enable_docs", False)]
)
def test_the_api_host_keeps_the_deployed_posture(key: str, expected: bool) -> None:
    docs = _docs()
    host = yaml.safe_load(_kind(docs, "ConfigMap")[0]["data"]["insight.yaml"])

    assert host["gears"]["api-gateway"]["config"][key] is expected


def test_the_service_listens_on_8085() -> None:
    docs = _docs()
    (port,) = _kind(docs, "Service")[0]["spec"]["ports"]

    assert port["port"] == 8085
    assert port["targetPort"] == "http"
