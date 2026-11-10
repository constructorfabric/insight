from __future__ import annotations

import base64
import json
import shutil
import subprocess
from pathlib import Path

import pytest
import yaml

GITOPS = Path(__file__).resolve().parents[5] / "deploy/gitops"
pytestmark = pytest.mark.skipif(shutil.which("helm") is None, reason="Helm is required to render local charts")


@pytest.fixture(params=["mock-source", "mock-airbyte-api"])
def chart(request: pytest.FixtureRequest, tmp_path: Path) -> Path:
    source = GITOPS / "environments/local" / request.param
    target = tmp_path / request.param
    shutil.copytree(source, target)
    return target


def render(chart: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [
            "helm",
            "template",
            chart.name,
            str(chart),
            "--namespace",
            "synthetic-app",
            "--set-string",
            "image=synthetic-toolbox:test",
            "--set-string",
            "upstream=http://synthetic-airbyte:8001",
        ],
        capture_output=True,
        text=True,
        check=False,
    )


def token_and_reference(chart: Path) -> tuple[str, dict[str, str]]:
    result = render(chart)
    assert result.returncode == 0, result.stderr
    resources = list(yaml.safe_load_all(result.stdout))
    secret = next(resource for resource in resources if resource["metadata"]["name"] == f"{chart.name}-control")
    deployment = next(resource for resource in resources if resource["kind"] == "Deployment")
    container = deployment["spec"]["template"]["spec"]["containers"][0]
    control = next(variable for variable in container["env"] if variable["name"] == "MOCK_CONTROL_TOKEN")
    token = base64.b64decode(secret["data"]["control_token"], validate=True).decode()
    return token, control["valueFrom"]["secretKeyRef"]


def substitute_lookup(chart: Path, secret: dict[str, object]) -> None:
    template = chart / "templates" / ("control.yaml" if chart.name == "mock-source" else "auth.yaml")
    body = template.read_text()
    lookup = 'lookup "v1" "Secret" .Release.Namespace $name'
    assert body.count(lookup) == 1
    replacement = f"({json.dumps(json.dumps(secret))} | fromJson)"
    template.write_text(body.replace(lookup, replacement))


def test_each_install_generates_a_random_control_token_and_wires_it_to_the_pod(chart: Path) -> None:
    first, reference = token_and_reference(chart)
    second, _ = token_and_reference(chart)
    assert len(first) == 64 and first.isalnum()
    assert len(second) == 64 and second.isalnum()
    assert first != second
    assert reference == {"name": f"{chart.name}-control", "key": "control_token"}
    assert "controlToken" not in yaml.safe_load((chart / "values.yaml").read_text())


def test_upgrades_reuse_the_token_returned_by_secret_lookup(chart: Path) -> None:
    expected = "synthetic-retained-control-token"
    substitute_lookup(chart, {"data": {"control_token": base64.b64encode(expected.encode()).decode()}})
    assert token_and_reference(chart)[0] == expected
    assert token_and_reference(chart)[0] == expected


@pytest.mark.parametrize("data", [{}, {"control_token": ""}])
def test_an_existing_secret_without_a_token_fails_instead_of_rotating(chart: Path, data: dict[str, str]) -> None:
    substitute_lookup(chart, {"data": data})
    result = render(chart)
    assert result.returncode != 0
    assert "existing control Secret must contain control_token" in result.stderr
