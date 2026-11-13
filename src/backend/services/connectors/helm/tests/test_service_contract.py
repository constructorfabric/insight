from __future__ import annotations

import re
from pathlib import Path

import pytest
import yaml

from .conftest import CONNECTORS_CHART, connectors_documents, render_connectors


def workload(documents: list[dict], kind: str) -> dict:
    return next(
        doc for doc in documents if doc["kind"] == kind and doc["metadata"]["name"] == "contract-test-connectors"
    )


def test_single_replica_recreates_with_its_service_account() -> None:
    documents = connectors_documents(CONNECTORS_CHART)
    deployment = workload(documents, "Deployment")
    assert deployment["spec"]["replicas"] == 1
    assert deployment["spec"]["strategy"] == {"type": "Recreate"}
    assert deployment["spec"]["template"]["spec"]["serviceAccountName"] == "contract-test-connectors"
    assert workload(documents, "Service")["spec"]["ports"][0]["port"] == 8088
    assert workload(documents, "ServiceAccount")["automountServiceAccountToken"] is True


@pytest.mark.parametrize("replicas", ["0", "2", "1.5", "invalid"])
def test_replica_count_other_than_one_is_refused(replicas: str) -> None:
    result = render_connectors(CONNECTORS_CHART, "--set", f"replicaCount={replicas}")
    assert result.returncode != 0, f"should reject replicaCount={replicas}"
    assert "must be exactly 1" in result.stderr


def test_migrate_runs_before_install_without_release_config_resources() -> None:
    documents = connectors_documents(CONNECTORS_CHART)
    job = next(doc for doc in documents if doc["kind"] == "Job")
    assert job["metadata"]["annotations"]["helm.sh/hook"] == "pre-install,pre-upgrade"
    assert job["metadata"]["annotations"]["helm.sh/hook-weight"] == "10"
    pod = job["spec"]["template"]["spec"]
    assert "serviceAccountName" not in pod
    assert "configMap" not in str(pod)
    command = pod["containers"][0]["command"][-1]
    assert "home_dir: /tmp/gears" in command
    assert "exec /app/connectors -c /tmp/migrate.yaml migrate" in command
    assert not any(doc["kind"] == "Secret" for doc in documents)


def test_service_route_and_database_always_render(connectors_umbrella: Path) -> None:
    documents = connectors_documents(connectors_umbrella, "-f", str(connectors_umbrella / "tests/values.yaml"))
    deployments = {doc["metadata"]["name"] for doc in documents if doc["kind"] == "Deployment"}
    assert "contract-test-connectors" in deployments
    services = {doc["metadata"]["name"] for doc in documents if doc["kind"] == "Service"}
    assert "contract-test-connectors" in services
    route_config = next(
        doc
        for doc in documents
        if doc["kind"] == "ConfigMap" and doc["metadata"]["name"] == "contract-test-gateway-routes"
    )
    routes = yaml.safe_load(route_config["data"]["routes.yaml"])["routes"]
    connector_routes = [route for route in routes if route["prefix"] == "/api/connectors"]
    assert len(connector_routes) == 1
    assert connector_routes[0] == {
        "prefix": "/api/connectors",
        "upstream": "http://contract-test-connectors:8088",
        "strip_prefix": True,
        "timeout_ms": 135000,
    }
    jobs = [doc for doc in documents if doc["kind"] == "Job"]
    assert any(doc["metadata"]["name"] == "contract-test-connectors-migrate" for doc in jobs)
    provisioner = next(doc for doc in jobs if doc["metadata"]["name"].endswith("mariadb-init-svcdbs"))
    script = provisioner["spec"]["template"]["spec"]["containers"][0]["command"][-1]
    assert 'CONNECTORS_DB="connectors"' in script


def test_umbrella_uses_existing_password_secrets_for_both_workloads(connectors_umbrella: Path) -> None:
    documents = connectors_documents(
        connectors_umbrella,
        "-f",
        str(connectors_umbrella / "tests/values.yaml"),
        "--set",
        "credentials.autoGenerate=false",
        "--set",
        "ingestion.reconcile.tenantId=11111111-2222-4333-8444-555555555555",
        "--set",
        "mariadb.host=database.example.test",
        "--set",
        "mariadb.port=3307",
        "--set",
        "mariadb.username=example-user",
        "--set",
        "mariadb.passwordSecret.name=example-db-creds",
        "--set",
        "mariadb.passwordSecret.key=example-db-password",
        "--set",
        "clickhouse.host=warehouse.example.test",
        "--set",
        "clickhouse.port=8124",
        "--set",
        "clickhouse.protocol=https",
        "--set",
        "clickhouse.username=example-reader",
        "--set",
        "clickhouse.passwordSecret.name=example-ch-creds",
        "--set",
        "clickhouse.passwordSecret.key=example-ch-password",
    )
    for kind, name in [("Deployment", "contract-test-connectors"), ("Job", "contract-test-connectors-migrate")]:
        doc = next(doc for doc in documents if doc["kind"] == kind and doc["metadata"]["name"] == name)
        container = doc["spec"]["template"]["spec"]["containers"][0]
        env = {item["name"].removeprefix("APP__gears__connectors__config__"): item for item in container["env"]}
        assert env["database_host"]["value"] == "database.example.test"
        assert env["database_port"]["value"] == "3307"
        assert env["database_user"]["value"] == "example-user"
        assert env["database_name"]["value"] == "connectors"
        assert env["database_password"]["valueFrom"]["secretKeyRef"] == {
            "name": "example-db-creds",
            "key": "example-db-password",
        }
        assert "database_url" not in env
        assert "envFrom" not in container or all("secretRef" not in item for item in container["envFrom"])
        if kind == "Deployment":
            assert env["reconcile_workflow_name"]["value"] == "contract-test-reconcile-loop"
            assert env["tenant_id"]["value"] == "11111111-2222-4333-8444-555555555555"
            assert env["clickhouse_url"]["value"] == "https://warehouse.example.test:8124"
            assert env["clickhouse_user"]["value"] == "example-reader"
            assert env["clickhouse_password"]["valueFrom"]["secretKeyRef"] == {
                "name": "example-ch-creds",
                "key": "example-ch-password",
            }
            assert env["identity_url"]["value"] == "http://contract-test-identity-resolution:8082"


@pytest.mark.parametrize("auto_generate", [True, False])
def test_connectors_templates_render_no_secrets(connectors_umbrella: Path, auto_generate: bool) -> None:
    result = render_connectors(
        connectors_umbrella,
        "-f",
        str(connectors_umbrella / "tests/values.yaml"),
        "--set",
        f"credentials.autoGenerate={str(auto_generate).lower()}",
    )
    assert result.returncode == 0, result.stderr
    for manifest in re.split(r"^---\s*$", result.stdout, flags=re.MULTILINE):
        document = yaml.safe_load(manifest)
        if not document or document["kind"] != "Secret":
            continue

        source = next(
            line.removeprefix("# Source: ") for line in manifest.splitlines() if line.startswith("# Source: ")
        )
        assert source != "insight/templates/connectors.yaml", (source, document["metadata"]["name"])
        assert not source.startswith("insight/charts/connectors/"), (source, document["metadata"]["name"])


def test_gears_host_verifies_gateway_identity(connectors_umbrella: Path) -> None:
    documents = connectors_documents(
        connectors_umbrella,
        "-f",
        str(connectors_umbrella / "tests/values.yaml"),
        "--set",
        "global.observability.logs.level=debug",
        "--set",
        "global.observability.logs.format=pretty",
    )
    configmap = next(
        doc
        for doc in documents
        if doc["kind"] == "ConfigMap" and doc["metadata"]["name"] == "contract-test-connectors-gears-config"
    )
    config = yaml.safe_load(configmap["data"]["insight.yaml"])
    assert config["logging"]["default"] == {"console_level": "debug", "console_format": "pretty"}
    assert config["gears"]["connectors"]["config"]["cluster"]["enabled"] is True
    assert "drift" not in config["gears"]["connectors"]["config"]
    assert config["gears"]["connectors"]["config"]["namespace"] == "default"
    host = config["gears"]["api-gateway"]["config"]
    assert host["bind_addr"] == "0.0.0.0:8088"
    assert host["metrics"]["prefix"] == "host"
    assert host["auth_disabled"] is False
    jwt = config["gears"]["oidc-authn-plugin"]["config"]["jwt"]
    assert jwt["claim_mapping"]["subject_tenant_id"] == "tenant_id"
    assert jwt["trusted_issuers"][0]["issuer"] == "https://contract-test-authenticator.default.svc.cluster.local:8443"


def test_role_only_patches_existing_connector_cronworkflows() -> None:
    documents = connectors_documents(CONNECTORS_CHART)
    assert workload(documents, "Role")["rules"] == [
        {"apiGroups": ["argoproj.io"], "resources": ["cronworkflows"], "verbs": ["get", "list", "watch", "patch"]}
    ]
    binding = workload(documents, "RoleBinding")
    assert binding["roleRef"] == {
        "apiGroup": "rbac.authorization.k8s.io",
        "kind": "Role",
        "name": "contract-test-connectors",
    }
    assert binding["subjects"] == [
        {"kind": "ServiceAccount", "name": "contract-test-connectors", "namespace": "default"}
    ]


@pytest.mark.parametrize("enabled", [False, True])
@pytest.mark.parametrize("tenant", ["", "example-tenant"])
def test_cluster_integration_requires_a_tenant_only_when_enabled(enabled: bool, tenant: str) -> None:
    result = render_connectors(
        CONNECTORS_CHART, "--set", f"cluster.enabled={str(enabled).lower()}", "--set-string", f"tenantId={tenant}"
    )
    if enabled and not tenant:
        assert result.returncode != 0
        assert "tenantId is required" in result.stderr
        return

    assert result.returncode == 0, result.stderr
    documents = [doc for doc in yaml.safe_load_all(result.stdout) if doc]
    configmap = next(doc for doc in documents if doc["kind"] == "ConfigMap")
    config = yaml.safe_load(configmap["data"]["insight.yaml"])["gears"]["connectors"]["config"]
    assert config["cluster"]["enabled"] is enabled
    container = workload(documents, "Deployment")["spec"]["template"]["spec"]["containers"][0]
    env = {entry["name"]: entry for entry in container["env"]}
    tenant_env = "APP__gears__connectors__config__tenant_id"
    if enabled:
        assert config["tenant_id"] == tenant
        assert env[tenant_env]["value"] == tenant
    else:
        assert "tenant_id" not in config
        assert tenant_env not in env
    for kind in ["Role", "RoleBinding"]:
        roles = [doc for doc in documents if doc["kind"] == kind]
        assert len(roles) == int(enabled)


def test_schedule_service_has_no_background_worker_or_retry_interval() -> None:
    service = Path(__file__).resolve().parents[2]
    gear = (service / "src/gear.rs").read_text()
    config = (service / "src/config.rs").read_text()
    assert "drift" not in gear
    assert "stateful" not in gear
    assert "RunnableCapability" not in gear
    assert "drift" not in config
    assert not (service / "src/domain/connector_schedules/drift.rs").exists()


def test_compose_mounts_the_authentication_ca_bundle_for_connectors() -> None:
    root = Path(__file__).resolve().parents[6]
    compose = yaml.safe_load((root / "docker-compose.yml").read_text())
    assert "./deploy/compose/authn-tls-certs:/certs:ro" in compose["services"]["connectors"]["volumes"]
