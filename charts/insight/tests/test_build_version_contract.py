from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

REPO = Path(__file__).resolve().parents[3]
SERVICES = REPO / "src/backend/services"

REQUIRED = {
    "analytics": ["existingSecret=cfg"],
    "identity-resolution": ["existingSecret=cfg", "gateway.issuer=https://issuer.test"],
}


def _render(chart: Path, *sets: str) -> list[dict]:
    args = ["helm", "template", "contract-test", str(chart)]
    for value in sets:
        args += ["--set", value]

    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        args,  # noqa: S607
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    assert proc.returncode == 0, f"render failed for {chart.name}: {proc.stderr}"

    return [doc for doc in yaml.safe_load_all(proc.stdout) if isinstance(doc, dict)]


def _app_version(chart: Path) -> str:
    return yaml.safe_load((chart / "Chart.yaml").read_text())["appVersion"]


def _service_container(docs: list[dict], name: str) -> dict:
    deployments = [d for d in docs if d.get("kind") == "Deployment"]
    assert len(deployments) == 1, f"expected one Deployment, got {len(deployments)}"

    containers = deployments[0]["spec"]["template"]["spec"]["containers"]
    matching = [c for c in containers if c["name"] == name]
    assert len(matching) == 1, f"expected one {name} container, got {len(matching)}"

    return matching[0]


def _env(container: dict) -> dict[str, str]:
    return {e["name"]: e["value"] for e in container.get("env", []) if "value" in e}


@pytest.mark.parametrize("service", sorted(REQUIRED))
def test_the_pod_reports_the_tag_it_was_pinned_to(service: str) -> None:
    chart = SERVICES / service / "helm"
    docs = _render(chart, *REQUIRED[service], "image.tag=0.0.0-test")

    env = _env(_service_container(docs, service))

    assert env["INSIGHT_BUILD_VERSION"] == "0.0.0-test"


@pytest.mark.parametrize("service", sorted(REQUIRED))
def test_an_unpinned_tag_reports_the_appversion_the_pipeline_bumped(service: str) -> None:
    chart = SERVICES / service / "helm"
    docs = _render(chart, *REQUIRED[service])

    env = _env(_service_container(docs, service))

    assert env["INSIGHT_BUILD_VERSION"] == _app_version(chart)
