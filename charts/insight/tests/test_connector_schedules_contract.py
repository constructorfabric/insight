from __future__ import annotations

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
    assert provision_env["CONNECTOR_SCHEDULES_ACCOUNT_USER"] == "insight_schedule"
    assert provision_env["CONNECTOR_SCHEDULES_SECRET_NAME"] == secret_name
    assert provision_env["CONNECTOR_SCHEDULES_SECRET_USER_KEY"] == user_key
    assert provision_env["CONNECTOR_SCHEDULES_SECRET_PASSWORD_KEY"] == password_key
    assert provision_env["INSIGHT_NAMESPACE"] == "synthetic"
    assert provision_env["INSIGHT_RELEASE"] == "contract-test"
    admin = next(entry for entry in provision["env"] if entry["name"] == "CONNECTOR_SCHEDULES_DB_PASSWORD")
    assert admin["valueFrom"] == {"secretKeyRef": {"name": "insight-db-creds", "key": "mariadb-root-password"}}

    role = resource(documents, "Role", hook_name)
    assert role["rules"] == [
        {"apiGroups": [""], "resources": ["secrets"], "resourceNames": [secret_name], "verbs": ["get"]},
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
