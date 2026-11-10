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


def resource(documents: list[dict], kind: str, suffix: str) -> dict:
    matches = [
        document for document in documents if document["kind"] == kind and document["metadata"]["name"].endswith(suffix)
    ]
    assert len(matches) == 1, f"expected one {suffix} {kind}"
    return matches[0]


def env(document: dict) -> dict[str, dict]:
    spec = document["spec"]
    workflow = spec.get("workflowSpec", spec)
    entries = workflow["templates"][0]["container"]["env"]
    return {entry["name"]: entry for entry in entries}


def test_planner_template_has_bounded_attempts_and_warehouse_credentials(chart_dependencies: None) -> None:
    documents = render()
    planner = resource(documents, "WorkflowTemplate", "-transform")
    reconcile = resource(documents, "CronWorkflow", "-reconcile-loop")
    workflow = planner["spec"]
    attempt = workflow["templates"][0]
    container = attempt["container"]
    values = yaml.safe_load((CHART / "values.yaml").read_text())
    version = yaml.safe_load((CHART / "Chart.yaml").read_text())["version"]

    assert values["ingestion"]["transformPlan"]["retry"] == {"attempts": 3, "wait": "15m"}
    assert planner["metadata"]["name"] == "contract-test-transform"
    assert planner["metadata"]["namespace"] == "example"
    assert planner["metadata"]["labels"]["app.kubernetes.io/component"] == "transform-plan"
    for key, value in reconcile["metadata"]["labels"].items():
        if key != "app.kubernetes.io/component":
            assert planner["metadata"]["labels"][key] == value, key
    assert INSTANCE_LABEL not in planner["metadata"]["labels"]
    assert "helm.sh/hook" not in planner["metadata"].get("annotations", {})
    assert "activeDeadlineSeconds" not in workflow
    assert attempt["activeDeadlineSeconds"] == 900
    assert attempt["retryStrategy"] == {"limit": 2, "retryPolicy": "Always", "backoff": {"duration": "15m"}}
    assert workflow["ttlStrategy"] == {"secondsAfterSuccess": 86400}
    assert workflow["podGC"] == {"strategy": "OnPodCompletion"}
    assert workflow["serviceAccountName"] == "argo-workflow"
    assert workflow["entrypoint"] == "run"
    assert workflow["synchronization"] == {"mutexes": [{"name": "contract-test-transform"}]}
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


@pytest.mark.parametrize("attempts", [1, 5])
def test_planner_honors_retry_resources_instance_and_connection_overrides(
    chart_dependencies: None, attempts: int
) -> None:
    documents = render(
        "--set",
        f"ingestion.transformPlan.retry.attempts={attempts}",
        "--set",
        "ingestion.transformPlan.retry.wait=2m",
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
    planner = resource(documents, "WorkflowTemplate", "-transform")
    hook = resource(documents, "Workflow", "-transform")
    reconcile = resource(documents, "CronWorkflow", "-reconcile-loop")
    workflow = planner["spec"]
    container = workflow["templates"][0]["container"]

    assert workflow["templates"][0]["retryStrategy"] == {
        "limit": attempts - 1,
        "retryPolicy": "Always",
        "backoff": {"duration": "2m"},
    }
    assert planner["metadata"]["labels"][INSTANCE_LABEL] == "example-controller"
    assert hook["metadata"]["labels"][INSTANCE_LABEL] == "example-controller"
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


def test_transform_hook_starts_after_every_other_release_hook(chart_dependencies: None) -> None:
    documents = render(
        "--set",
        "clickhouse.fieldValueMap.enabled=true",
        "--set",
        "keycloakConfig.enabled=true",
        "--set",
        "keycloakConfig.url=https://idp.example.test",
    )
    hook = resource(documents, "Workflow", "-transform")
    planner = resource(documents, "WorkflowTemplate", "-transform")
    annotations = hook["metadata"]["annotations"]

    assert hook["metadata"]["name"] == "contract-test-transform"
    assert hook["metadata"]["namespace"] == "example"
    assert hook["metadata"]["labels"] == planner["metadata"]["labels"]
    assert INSTANCE_LABEL not in hook["metadata"]["labels"]
    assert annotations["helm.sh/hook"] == "post-install,post-upgrade,post-rollback"
    assert annotations["helm.sh/hook-delete-policy"] == "before-hook-creation"
    assert hook["spec"] == {"workflowTemplateRef": {"name": planner["metadata"]["name"]}}
    for event in ("post-install", "post-upgrade"):
        other_weights = []
        for document in documents:
            if document is hook:
                continue
            other_annotations = document["metadata"].get("annotations", {})
            if event in other_annotations.get("helm.sh/hook", "").split(","):
                other_weights.append(int(other_annotations.get("helm.sh/hook-weight", "0")))

        assert other_weights, f"expected other {event} hooks"
        assert int(annotations["helm.sh/hook-weight"]) > max(other_weights), event


def test_planning_has_no_schedule(chart_dependencies: None) -> None:
    documents = render()
    assert not any(
        document["kind"] == "CronWorkflow" and document["metadata"]["name"].endswith("-transform-plan")
        for document in documents
    )
    values = yaml.safe_load((CHART / "values.yaml").read_text())
    assert "schedule" not in values["ingestion"]["transformPlan"]


@pytest.mark.parametrize("gate", ["ingestion.templates.enabled", "ingestion.transformPlan.enabled"])
def test_disabled_planning_renders_no_planner(chart_dependencies: None, gate: str) -> None:
    documents = render("--set", f"{gate}=false")
    assert not any(
        document["metadata"]["name"].endswith(("-transform", "-transform-plan")) for document in documents
    ), gate


def test_reconcile_no_longer_carries_the_planner_release(chart_dependencies: None) -> None:
    assert "INSIGHT_VERSION" not in env(resource(render(), "CronWorkflow", "-reconcile-loop"))
