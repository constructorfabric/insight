from __future__ import annotations

import hashlib
import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]


def render(*overrides: str, chart: Path = CHART) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "helm",
            "template",
            "contract-test",
            str(chart),
            "--namespace",
            "synthetic",
            "-f",
            str(chart / "tests/values.yaml"),
            *overrides,
        ],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


def resource(documents: list[dict], kind: str, name: str) -> dict:
    return next(doc for doc in documents if doc and doc["kind"] == kind and doc["metadata"]["name"] == name)


@pytest.mark.parametrize("mode", ["helm", "gitops"])
@pytest.mark.parametrize("supplied", [False, True])
def test_runtime_provisioning_owns_credentials_and_honours_supplied_secret_names(mode: str, supplied: bool) -> None:
    secret_name, user_key, password_key = (
        ("example-db-creds", "user", "pass") if supplied else ("insight-schedule-db", "username", "password")
    )
    result = render(
        "--set",
        f"credentials.deploymentMode={mode}",
        "--set",
        f"credentials.autoGenerate={'true' if mode == 'helm' else 'false'}",
        "--set",
        f"global.connectorSchedules.database.secretName={secret_name}",
        "--set",
        f"global.connectorSchedules.database.userKey={user_key}",
        "--set",
        f"global.connectorSchedules.database.passwordKey={password_key}",
    )
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    assert not any(
        doc
        and doc["kind"] == "Secret"
        and doc["metadata"]["name"] in {secret_name, "insight-schedule-db-provision-env"}
        for doc in documents
    )
    cron = resource(documents, "CronWorkflow", "contract-test-reconcile-loop")
    workflow = cron["spec"]["workflowSpec"]
    container = workflow["templates"][0]["container"]
    env = {entry["name"]: entry.get("value") for entry in container["env"]}
    assert env["CONNECTOR_SCHEDULES_DB_HOST"] == "mariadb.example.test"
    assert env["CONNECTOR_SCHEDULES_DB_NAME"] == "connectors"
    references = {e["name"]: e.get("valueFrom") for e in container["env"]}
    for name, key in [("USER", user_key), ("PASSWORD", password_key)]:
        assert references["CONNECTOR_SCHEDULES_DB_" + name] == {"secretKeyRef": {"name": secret_name, "key": key}}

    hook_name = "contract-test-schedule-db-provision"
    hook = resource(documents, "Job", hook_name)
    assert hook["metadata"]["annotations"]["helm.sh/hook-weight"] == "15"
    assert hook["metadata"]["annotations"]["helm.sh/hook"] == "pre-install,pre-upgrade"
    pod = hook["spec"]["template"]["spec"]
    assert pod["serviceAccountName"] == hook_name
    provision = pod["containers"][0]
    assert provision["command"][-1].endswith("provision_schedule_secret.py")
    provision_env = {entry["name"]: entry.get("value") for entry in provision["env"]}
    assert "CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD" not in provision_env
    assert provision_env["CONNECTOR_SCHEDULES_ACCOUNT_USER"] == "insight_schedule_connectors"
    assert provision_env["CONNECTOR_SCHEDULES_SECRET_NAME"] == secret_name
    assert provision_env["CONNECTOR_SCHEDULES_SECRET_USER_KEY"] == user_key
    assert provision_env["CONNECTOR_SCHEDULES_SECRET_PASSWORD_KEY"] == password_key
    assert provision_env["CONNECTOR_SCHEDULES_DB_NAME"] == "connectors"
    assert provision_env["INSIGHT_NAMESPACE"] == "synthetic"
    assert provision_env["INSIGHT_RELEASE"] == "contract-test"
    admin = next(entry for entry in provision["env"] if entry["name"] == "CONNECTOR_SCHEDULES_DB_PASSWORD")
    assert admin["valueFrom"] == {"secretKeyRef": {"name": "insight-db-creds", "key": "mariadb-root-password"}}

    role = resource(documents, "Role", hook_name)
    assert role["rules"] == [
        {"apiGroups": [""], "resources": ["secrets"], "resourceNames": [secret_name], "verbs": ["get", "patch"]},
        {"apiGroups": [""], "resources": ["secrets"], "verbs": ["create"]},
    ]
    binding = resource(documents, "RoleBinding", hook_name)
    assert binding["roleRef"] == {"apiGroup": "rbac.authorization.k8s.io", "kind": "Role", "name": hook_name}
    assert binding["subjects"] == [{"kind": "ServiceAccount", "name": hook_name, "namespace": "synthetic"}]
    for kind in ["ServiceAccount", "Role", "RoleBinding"]:
        metadata = resource(documents, kind, hook_name)["metadata"]
        assert metadata["namespace"] == "synthetic"
        assert metadata["annotations"] == {
            "helm.sh/hook": "pre-install,pre-upgrade",
            "helm.sh/hook-weight": "-5",
            "helm.sh/hook-delete-policy": "before-hook-creation",
        }


@pytest.mark.parametrize("enabled", [False, True])
@pytest.mark.parametrize("database", ["connectors", "example_connectors"])
def test_schedule_reader_and_account_hook_follow_ingestion_templates(enabled: bool, database: str) -> None:
    result = render(
        "--set", f"ingestion.templates.enabled={str(enabled).lower()}", "--set", f"connectors.databaseName={database}"
    )
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    hooks = [doc for doc in documents if doc and doc["metadata"].get("name") == "contract-test-schedule-db-provision"]
    if not enabled:
        assert "CONNECTOR_SCHEDULES_" not in result.stdout
        assert not any(
            doc and doc["kind"] == "CronWorkflow" and doc["metadata"]["name"] == "contract-test-reconcile-loop"
            for doc in documents
        )
        assert hooks == []
        return

    cron = resource(documents, "CronWorkflow", "contract-test-reconcile-loop")
    env = cron["spec"]["workflowSpec"]["templates"][0]["container"]["env"]
    schedule_env = {
        entry["name"]: entry.get("value") for entry in env if entry["name"].startswith("CONNECTOR_SCHEDULES_")
    }
    assert schedule_env["CONNECTOR_SCHEDULES_DB_NAME"] == database
    hook = resource(hooks, "Job", "contract-test-schedule-db-provision")
    assert int(hook["metadata"]["annotations"]["helm.sh/hook-weight"]) > 10
    hook_env = hook["spec"]["template"]["spec"]["containers"][0]["env"]
    assert next(entry["value"] for entry in hook_env if entry["name"] == "CONNECTOR_SCHEDULES_DB_NAME") == database


def test_chart_renders_without_ingestion_templates() -> None:
    result = render("--set", "ingestion.templates.enabled=false")
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("enabled", [False, True])
@pytest.mark.parametrize("tenant", ["", "example-tenant"])
def test_cluster_integration_follows_ingestion_templates(enabled: bool, tenant: str) -> None:
    result = render(
        "--set",
        f"ingestion.templates.enabled={str(enabled).lower()}",
        "--set-string",
        f"ingestion.reconcile.tenantId={tenant}",
    )
    if enabled and not tenant:
        assert result.returncode != 0
        assert "tenantId is required" in result.stderr
        return

    assert result.returncode == 0, result.stderr
    documents = [doc for doc in yaml.safe_load_all(result.stdout) if doc]
    configmap = resource(documents, "ConfigMap", "contract-test-connectors-gears-config")
    config = yaml.safe_load(configmap["data"]["insight.yaml"])["gears"]["connectors"]["config"]
    assert config["cluster"]["enabled"] is enabled
    deployment = resource(documents, "Deployment", "contract-test-connectors")
    env = {entry["name"]: entry for entry in deployment["spec"]["template"]["spec"]["containers"][0]["env"]}
    tenant_env = "APP__gears__connectors__config__tenant_id"
    if enabled:
        assert config["tenant_id"] == tenant
        assert env[tenant_env]["value"] == tenant
    else:
        assert "tenant_id" not in config
        assert tenant_env not in env
    for kind in ["Role", "RoleBinding"]:
        roles = [
            doc for doc in documents if doc["kind"] == kind and doc["metadata"]["name"] == "contract-test-connectors"
        ]
        assert len(roles) == int(enabled)


def test_functional_ci_environment_renders_without_ingestion_templates() -> None:
    environment = CHART.parents[1] / "deploy/gitops/environments/functional-ci/values.yaml"
    result = subprocess.run(
        ["helm", "template", "contract-test", str(CHART), "-f", str(environment)],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    resource(documents, "Deployment", "contract-test-connectors")


@pytest.mark.parametrize("database", ["example_connectors", "a" * 63, "a" * 63 + "b", "a" * 63 + "c"])
@pytest.mark.parametrize("username", ["", "example_explicit", "u" * 80])
def test_schedule_account_names_follow_the_installation_database_and_explicit_override(
    database: str, username: str
) -> None:
    result = render(
        "--set",
        f"connectors.databaseName={database}",
        "--set-string",
        f"global.connectorSchedules.database.username={username}",
    )
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    hook = resource(documents, "Job", "contract-test-schedule-db-provision")
    env = {entry["name"]: entry.get("value") for entry in hook["spec"]["template"]["spec"]["containers"][0]["env"]}
    expected = username or "insight_schedule_" + database
    if len(expected) > 80:
        expected = expected[:67] + "_" + hashlib.sha256(expected.encode()).hexdigest()[:12]
    assert env["CONNECTOR_SCHEDULES_ACCOUNT_USER"] == expected
    assert len(expected) <= 80


@pytest.mark.parametrize("username", ["a" * 81, "invalid-name"])
def test_invalid_explicit_schedule_usernames_fail_rendering(username: str) -> None:
    result = render("--set", f"global.connectorSchedules.database.username={username}")
    assert result.returncode != 0
    assert "username must contain 1 to 80" in result.stderr


def test_save_budget_fits_the_chart_and_local_in_process_gateway_timeout() -> None:
    result = render()
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    configmap = resource(documents, "ConfigMap", "contract-test-connectors-gears-config")
    chart = yaml.safe_load(configmap["data"]["insight.yaml"])
    local = yaml.safe_load((CHART.parents[1] / "src/backend/services/connectors/config/insight.yaml").read_text())
    for config in [chart, local]:
        assert config["gears"]["api-gateway"]["config"]["defaults"]["request_timeout"] == "60s"
