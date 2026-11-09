from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]
INSTANCE_LABEL = "workflows.argoproj.io/controller-instanceid"


@pytest.fixture(scope="session")
def chart_dependencies() -> None:
    result = subprocess.run(
        ["helm", "dependency", "update", str(CHART)], capture_output=True, text=True, timeout=300, check=False
    )
    assert result.returncode == 0, result.stderr


def render(*overrides: str) -> list[dict]:
    result = subprocess.run(
        [
            "helm",
            "template",
            "contract-test",
            str(CHART),
            "--namespace",
            "example",
            "--values",
            str(CHART / "tests" / "values.yaml"),
            *overrides,
        ],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    return [document for document in yaml.safe_load_all(result.stdout) if document]


def cron(documents: list[dict], suffix: str) -> dict:
    matches = [
        document
        for document in documents
        if document["kind"] == "CronWorkflow" and document["metadata"]["name"].endswith(suffix)
    ]
    assert len(matches) == 1, f"expected one {suffix} CronWorkflow"
    return matches[0]


def env(document: dict) -> dict[str, dict]:
    entries = document["spec"]["workflowSpec"]["templates"][0]["container"]["env"]
    return {entry["name"]: entry for entry in entries}


def test_planner_has_an_independent_bounded_loop_and_warehouse_credentials(chart_dependencies: None) -> None:
    documents = render()
    planner = cron(documents, "-transform-plan")
    reconcile = cron(documents, "-reconcile-loop")
    spec = planner["spec"]
    workflow = spec["workflowSpec"]
    container = workflow["templates"][0]["container"]
    values = yaml.safe_load((CHART / "values.yaml").read_text())
    version = yaml.safe_load((CHART / "Chart.yaml").read_text())["version"]

    assert planner["metadata"]["name"] == "contract-test-transform-plan"
    assert planner["metadata"]["namespace"] == "example"
    assert planner["metadata"]["labels"]["app.kubernetes.io/component"] == "transform-plan"
    for key, value in reconcile["metadata"]["labels"].items():
        if key != "app.kubernetes.io/component":
            assert planner["metadata"]["labels"][key] == value, key
    assert INSTANCE_LABEL not in planner["metadata"]["labels"]
    assert spec["schedules"] == ["*/15 * * * *"]
    assert spec["timezone"] == "UTC"
    assert spec["concurrencyPolicy"] == "Forbid"
    assert workflow["activeDeadlineSeconds"] == 900
    assert workflow["serviceAccountName"] == "argo-workflow"
    assert workflow["entrypoint"] == "run"
    assert "synchronization" not in yaml.safe_dump(planner)
    assert workflow["templates"][0]["name"] == "run"
    assert container["command"] == ["bash", "/ingestion/reconcile-connectors/transform-plan.sh"]
    assert container["image"] == values["ingestion"]["toolboxImage"]
    assert container["resources"] == {
        "requests": {"cpu": "100m", "memory": "256Mi"},
        "limits": {"cpu": "500m", "memory": "512Mi"},
    }
    expected = {name: entry for name, entry in env(reconcile).items() if name.startswith("RECONCILE_DEST_CLICKHOUSE_")}
    expected.update(
        INSIGHT_VERSION={"name": "INSIGHT_VERSION", "value": version},
        RECONCILE_RUN_ID={"name": "RECONCILE_RUN_ID", "value": "{{workflow.name}}"},
        CONNECTORS_DIR=env(reconcile)["CONNECTORS_DIR"],
        INSIGHT_LOG_LEVEL=env(reconcile)["INSIGHT_LOG_LEVEL"],
    )
    assert env(planner) == expected
    assert (
        env(planner)["RECONCILE_DEST_CLICKHOUSE_PASSWORD"]["valueFrom"]["secretKeyRef"]
        == values["clickhouse"]["passwordSecret"]
    )


def test_planner_honors_schedule_resources_instance_and_connection_overrides(chart_dependencies: None) -> None:
    documents = render(
        "--set-string",
        "ingestion.transformPlan.schedule=7 * * * *",
        "--set",
        "ingestion.transformPlan.resources.requests.cpu=200m",
        "--set",
        "ingestion.transformPlan.resources.requests.memory=384Mi",
        "--set",
        "ingestion.transformPlan.resources.limits.cpu=750m",
        "--set",
        "ingestion.transformPlan.resources.limits.memory=768Mi",
        "--set",
        "ingestion.reconcile.argoInstanceId=example-controller",
        "--set",
        "global.imagePullSecrets[0].name=example-registry",
        "--set",
        "ingestion.toolboxImage=example/toolbox:test",
        "--set",
        "clickhouse.host=warehouse.example.test",
        "--set",
        "clickhouse.port=8443",
        "--set",
        "clickhouse.protocol=https",
        "--set",
        "clickhouse.database=example_database",
        "--set",
        "clickhouse.username=example-user",
        "--set",
        "clickhouse.passwordSecret.name=example-warehouse",
        "--set",
        "clickhouse.passwordSecret.key=example-password",
        "--set",
        "clickhouse.clusterMode=true",
        "--set",
        "clickhouse.clusterName=example_cluster",
    )
    planner = cron(documents, "-transform-plan")
    reconcile = cron(documents, "-reconcile-loop")
    workflow = planner["spec"]["workflowSpec"]
    container = workflow["templates"][0]["container"]

    assert planner["spec"]["schedules"] == ["7 * * * *"]
    assert planner["metadata"]["labels"][INSTANCE_LABEL] == "example-controller"
    assert workflow["imagePullSecrets"] == [{"name": "example-registry"}]
    assert container["image"] == "example/toolbox:test"
    assert container["resources"] == {
        "requests": {"cpu": "200m", "memory": "384Mi"},
        "limits": {"cpu": "750m", "memory": "768Mi"},
    }
    for name, entry in env(reconcile).items():
        if name.startswith("RECONCILE_DEST_CLICKHOUSE_"):
            assert env(planner)[name] == entry, name
    assert env(planner)["RECONCILE_DEST_CLICKHOUSE_PASSWORD"]["valueFrom"]["secretKeyRef"] == {
        "name": "example-warehouse",
        "key": "example-password",
    }


@pytest.mark.parametrize("gate", ["ingestion.templates.enabled", "ingestion.transformPlan.enabled"])
def test_disabled_planning_renders_no_planner(chart_dependencies: None, gate: str) -> None:
    documents = render("--set", f"{gate}=false")
    assert not any(document["metadata"]["name"].endswith("-transform-plan") for document in documents), gate


def test_reconcile_no_longer_carries_the_planner_release(chart_dependencies: None) -> None:
    assert "INSIGHT_VERSION" not in env(cron(render(), "-reconcile-loop"))
