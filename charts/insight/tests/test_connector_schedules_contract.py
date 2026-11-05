from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]
CADENCE_KEY = "APP__gears__analytics__config__connector_schedules__reconcile_schedules"
READER_KEY = "authenticator.serviceTokens.services.reconcile.public_keys={synthetic-public-key}"


def render(*overrides: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "helm",
            "template",
            "contract-test",
            str(CHART),
            "--namespace",
            "synthetic",
            "-f",
            str(CHART / "tests/values.yaml"),
            *overrides,
        ],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


def enabled(*overrides: str) -> subprocess.CompletedProcess[str]:
    return render(
        "--set",
        "global.connectorSchedules.enabled=true",
        "--set",
        "authenticator.serviceTokens.services.reconcile.roles[0]=connector-schedules-reader",
        "--set",
        READER_KEY,
        *overrides,
    )


def resource(documents: list[dict], kind: str, name: str) -> dict:
    return next(doc for doc in documents if doc and doc["kind"] == kind and doc["metadata"]["name"] == name)


@pytest.mark.parametrize("gitops", [False, True])
def test_one_switch_enables_the_api_and_its_reconcile_reader(gitops: bool) -> None:
    overrides = ["--set", "credentials.autoGenerate=false"] if gitops else []
    result = enabled(*overrides)
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    config = resource(documents, "ConfigMap", "contract-test-analytics-gears-config")
    gear = yaml.safe_load(config["data"]["analytics.yaml"])
    assert gear["gears"]["analytics"]["config"]["connector_schedules"]["enabled"] is True

    cron = resource(documents, "CronWorkflow", "contract-test-reconcile-loop")
    workflow = cron["spec"]["workflowSpec"]
    container = workflow["templates"][0]["container"]
    env = {entry["name"]: entry.get("value") for entry in container["env"]}
    service = resource(documents, "Service", "contract-test-analytics")
    port = service["spec"]["ports"][0]["port"]
    assert env["CONNECTOR_SCHEDULES_URL"] == f"http://{service['metadata']['name']}:{port}/v1/connector-schedules"
    assert env["CONNECTOR_SCHEDULES_ENABLED"] == "true"
    assert env["SERVICE_TOKEN_NAME"] == "reconcile"
    assert env["SERVICE_TOKEN_TENANT_ID"] == "00000000-0000-4000-8000-000000000001"
    assert env["SERVICE_TOKEN_KEY_FILE"] == "/etc/insight/reconcile/private.pem"
    assert workflow["volumes"] == [
        {
            "name": "service-token-key",
            "secret": {
                "secretName": "insight-reconcile-signing-key",
                "defaultMode": 0o444,
                "items": [{"key": "private.pem", "path": "private.pem"}],
            },
        }
    ]
    assert container["volumeMounts"] == [
        {"name": "service-token-key", "mountPath": "/etc/insight/reconcile", "readOnly": True}
    ]
    if not gitops:
        secret = resource(documents, "Secret", "insight-authenticator-config")["stringData"]
        assert env["SERVICE_TOKEN_URL"] == secret["APP__gears__authenticator__config__service_tokens__audience"]


def test_reported_reconcile_cadence_is_the_cronworkflow_cadence() -> None:
    result = enabled("--set-string", "ingestion.reconcile.schedule=7 */2 * * *")
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    cron = resource(documents, "CronWorkflow", "contract-test-reconcile-loop")
    secret = resource(documents, "Secret", "insight-analytics-config")
    assert json.loads(secret["stringData"][CADENCE_KEY]) == cron["spec"]["schedules"] == ["7 */2 * * *"]


def test_authenticator_tenant_override_and_existing_key_secret_are_used() -> None:
    result = enabled(
        "--set",
        "authenticator.oidc.defaultTenantId=00000000-0000-4000-8000-000000000002",
        "--set",
        "ingestion.reconcile.serviceTokenKeySecret=synthetic-signing-key",
    )
    assert result.returncode == 0, result.stderr
    cron = resource(list(yaml.safe_load_all(result.stdout)), "CronWorkflow", "contract-test-reconcile-loop")
    workflow = cron["spec"]["workflowSpec"]
    env = workflow["templates"][0]["container"]["env"]
    assert next(e["value"] for e in env if e["name"] == "SERVICE_TOKEN_TENANT_ID").endswith("000000000002")
    assert workflow["volumes"][0]["secret"]["secretName"] == "synthetic-signing-key"


def test_the_default_switch_adds_no_reader_environment_or_key_mount() -> None:
    result = render()
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    workflow = resource(documents, "CronWorkflow", "contract-test-reconcile-loop")["spec"]["workflowSpec"]
    container = workflow["templates"][0]["container"]
    assert "volumes" not in workflow
    assert "volumeMounts" not in container
    assert not any(e["name"].startswith(("SERVICE_TOKEN_", "CONNECTOR_SCHEDULES_")) for e in container["env"])
    gear = resource(documents, "ConfigMap", "contract-test-analytics-gears-config")["data"]["analytics.yaml"]
    assert yaml.safe_load(gear)["gears"]["analytics"]["config"]["connector_schedules"]["enabled"] is False


@pytest.mark.parametrize(
    ("override", "message"),
    [
        ("global.tenantDefaultId=", "authenticator.oidc.defaultTenantId or global.tenantDefaultId"),
        ("global.tenantDefaultId=invalid", "authenticator.oidc.defaultTenantId or global.tenantDefaultId"),
        ("ingestion.reconcile.serviceTokenKeySecret=", "ingestion.reconcile.serviceTokenKeySecret"),
        ("authenticator.serviceTokens.services.reconcile=null", "authenticator.serviceTokens.services.reconcile"),
        (
            "authenticator.serviceTokens.services.reconcile.roles[0]=other",
            "authenticator.serviceTokens.services.reconcile.roles",
        ),
        ("ingestion.templates.enabled=false", "ingestion.templates.enabled=true"),
    ],
)
@pytest.mark.parametrize("autogenerate", ["true", "false"])
def test_enabled_stands_refuse_incomplete_configuration(override: str, message: str, autogenerate: str) -> None:
    result = enabled("--set", f"credentials.autoGenerate={autogenerate}", "--set", override)
    assert result.returncode != 0, override
    assert message in result.stderr, result.stderr


def test_local_template_opts_in_and_registers_the_reader() -> None:
    local = CHART.parents[1] / "deploy/gitops/environments/local/values.yaml.template"
    result = render("-f", str(local), "--set", READER_KEY)
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    auth = resource(documents, "ConfigMap", "contract-test-authenticator-gears-config")
    config = yaml.safe_load(auth["data"]["authenticator.yaml"])
    entry = config["gears"]["authenticator"]["config"]["service_tokens"]["services"]["reconcile"]
    assert entry["roles"] == ["connector-schedules-reader"]


def render_reader_key(field: str, value: list[str] | None, tmp_path: Path) -> subprocess.CompletedProcess[str]:
    entry = {"roles": ["connector-schedules-reader"], field: value}
    config = {"authenticator": {"serviceTokens": {"services": {"reconcile": entry}}}}
    values = tmp_path / "keys.yaml"
    values.write_text(yaml.safe_dump(config))

    return render("--set", "global.connectorSchedules.enabled=true", "-f", str(values))


@pytest.mark.parametrize("field", ["public_keys", "public_key_paths"])
@pytest.mark.parametrize("value", [[], None, [""], ["   "]])
def test_reader_requires_a_nonblank_key(field: str, value: list[str] | None, tmp_path: Path) -> None:
    result = render_reader_key(field, value, tmp_path)
    assert result.returncode != 0, (field, value)
    assert "authenticator.serviceTokens.services.reconcile.public_keys" in result.stderr
    assert "authenticator.serviceTokens.services.reconcile.public_key_paths" in result.stderr


@pytest.mark.parametrize("field", ["public_keys", "public_key_paths"])
def test_reader_accepts_a_nonblank_key(field: str, tmp_path: Path) -> None:
    result = render_reader_key(field, ["", "synthetic-key"], tmp_path)
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("minimum", [None, 60])
def test_schedule_minimum_is_rendered_in_the_analytics_config(minimum: int | None) -> None:
    overrides = [] if minimum is None else ["--set", f"analytics.connectorSchedules.minIntervalMinutes={minimum}"]
    result = render(*overrides)
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    config = resource(documents, "ConfigMap", "contract-test-analytics-gears-config")
    gear = yaml.safe_load(config["data"]["analytics.yaml"])
    assert gear["gears"]["analytics"]["config"]["connector_schedules"]["min_interval_minutes"] == (
        30 if minimum is None else minimum
    )
