from __future__ import annotations

import json
import subprocess
from pathlib import Path

import yaml

CHART = Path(__file__).resolve().parents[1]


def render(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["helm", "template", "mock-source", str(CHART), "--namespace", "synthetic-tests", *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


def test_local_chart_connects_both_instances_to_the_bounded_toolbox_service() -> None:
    result = render("--set-string", "image=synthetic-toolbox:test")
    assert result.returncode == 0, result.stderr
    objects = {(item["kind"], item["metadata"]["name"]): item for item in yaml.safe_load_all(result.stdout)}
    pod = objects["Deployment", "mock-source"]["spec"]["template"]["spec"]
    container = pod["containers"][0]
    assert container["image"] == "synthetic-toolbox:test"
    assert container["command"] == ["python3", "/ingestion/tools/mock-source/server.py"]
    assert container["resources"]["limits"]["memory"] == "256Mi"
    assert container["readinessProbe"]["httpGet"]["path"] == "/health"
    assert pod["automountServiceAccountToken"] is False
    assert objects["Service", "mock-source"]["spec"]["selector"] == {"app.kubernetes.io/name": "mock-source"}
    scenarios = json.loads(objects["ConfigMap", "mock-source-scenarios"]["data"]["scenarios.json"])
    for instance in ("mock-a", "mock-b"):
        secret = objects["Secret", instance]
        assert secret["metadata"]["labels"]["app.kubernetes.io/part-of"] == "insight"
        assert secret["metadata"]["annotations"] == {
            "insight.cyberfabric.com/connector": "mock",
            "insight.cyberfabric.com/source-id": instance,
        }
        assert secret["stringData"]["base_url"] == (
            f"http://mock-source.synthetic-tests.svc.cluster.local:8080/instances/{instance}"
        )
        assert secret["stringData"]["api_token"] == objects["Secret", "mock-source-auth"]["stringData"]["api_token"]
        assert set(scenarios[instance]["streams"]) == {"mock_events", "mock_updates"}


def test_local_chart_refuses_a_missing_toolbox_image() -> None:
    result = render()
    assert result.returncode != 0
    assert "image is required" in result.stderr


def test_install_guard_refuses_other_environments_before_cluster_access() -> None:
    result = subprocess.run(
        ["make", "local-mock-source-guard", "ENV=synthetic-other", "NS_APP=synthetic-tests"],
        cwd=CHART.parents[2],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode != 0
    assert "mock-source is local-only" in result.stdout
