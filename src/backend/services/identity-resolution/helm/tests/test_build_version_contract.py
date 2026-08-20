from __future__ import annotations

import subprocess
from pathlib import Path

import yaml

SUBCHART = Path(__file__).resolve().parents[1]
BASE = [
    "--set",
    "existingSecret=test-secret",
    "--set",
    "gateway.issuer=https://issuer.test",
]


def _service_env(*extra: str) -> dict[str, str]:
    proc = subprocess.run(  # noqa: S603 — test-controlled argv
        ["helm", "template", "contract-test", str(SUBCHART), *BASE, *extra],  # noqa: S607
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    assert proc.returncode == 0, f"render failed: {proc.stderr}"

    docs = [d for d in yaml.safe_load_all(proc.stdout) if isinstance(d, dict)]
    deployments = [d for d in docs if d.get("kind") == "Deployment"]
    assert len(deployments) == 1, f"expected one Deployment, got {len(deployments)}"

    containers = deployments[0]["spec"]["template"]["spec"]["containers"]
    service = [c for c in containers if c["name"] == "identity-resolution"]
    assert len(service) == 1, f"expected one service container, got {len(service)}"

    return {
        entry["name"]: entry["value"]
        for entry in service[0].get("env", [])
        if "value" in entry
    }


def _chart_app_version() -> str:
    return yaml.safe_load((SUBCHART / "Chart.yaml").read_text())["appVersion"]


def test_the_pod_reports_the_tag_it_was_pinned_to() -> None:
    env = _service_env("--set", "image.tag=0.0.0-test")

    assert env["INSIGHT_BUILD_VERSION"] == "0.0.0-test"


def test_an_unpinned_tag_reports_the_appversion_the_pipeline_bumped() -> None:
    env = _service_env()

    assert env["INSIGHT_BUILD_VERSION"] == _chart_app_version()
