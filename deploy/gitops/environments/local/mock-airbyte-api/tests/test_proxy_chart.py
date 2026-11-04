from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]


def render(*arguments: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["helm", "template", "mock-airbyte-api", str(CHART), "--namespace", "synthetic-app", *arguments],
        capture_output=True,
        text=True,
        check=False,
    )


def test_proxy_chart_uses_toolbox_direct_upstream_and_separate_auth() -> None:
    result = render(
        "--set-string", "image=synthetic-toolbox:test", "--set-string", "upstream=http://synthetic.invalid:8001"
    )
    assert result.returncode == 0, result.stderr
    objects = {(item["kind"], item["metadata"]["name"]): item for item in yaml.safe_load_all(result.stdout)}
    pod = objects["Deployment", "mock-airbyte-api"]["spec"]["template"]["spec"]
    container = pod["containers"][0]
    assert container["image"] == "synthetic-toolbox:test"
    assert container["command"] == ["python3", "/ingestion/tools/mock-airbyte-api/server.py"]
    env = {entry["name"]: entry for entry in container["env"]}
    assert env["MOCK_AIRBYTE_UPSTREAM"]["value"] == "http://synthetic.invalid:8001"
    assert env["MOCK_CONTROL_TOKEN"]["valueFrom"]["secretKeyRef"] == {
        "name": "mock-airbyte-api-auth",
        "key": "control_token",
    }
    assert pod["automountServiceAccountToken"] is False
    assert container["securityContext"]["readOnlyRootFilesystem"] is True
    assert container["resources"]["limits"]["memory"] == "256Mi"
    assert objects["Service", "mock-airbyte-api"]["spec"]["selector"] == {"app.kubernetes.io/name": "mock-airbyte-api"}
    assert objects["Secret", "mock-airbyte-api-auth"]["stringData"]["control_token"] == "synthetic-control-token"


@pytest.mark.parametrize(
    "arguments,message",
    [([], "image is required"), (["--set-string", "image=synthetic-toolbox:test"], "upstream is required")],
)
def test_proxy_chart_requires_runtime_inputs(arguments: list[str], message: str) -> None:
    result = render(*arguments)
    assert result.returncode != 0 and message in result.stderr


def test_proxy_chart_can_isolate_a_second_upstream_under_another_release() -> None:
    result = subprocess.run(
        [
            "helm",
            "template",
            "synthetic-secondary",
            str(CHART),
            "--set-string",
            "image=synthetic-toolbox:test",
            "--set-string",
            "upstream=http://synthetic.invalid:8080",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    objects = list(yaml.safe_load_all(result.stdout))
    assert {item["metadata"]["name"] for item in objects} == {"synthetic-secondary", "synthetic-secondary-auth"}
    deployment = next(item for item in objects if item["kind"] == "Deployment")
    assert (
        deployment["spec"]["template"]["spec"]["containers"][0]["command"][-1]
        == "/ingestion/tools/mock-airbyte-api/server.py"
    )
