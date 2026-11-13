from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[6]
CONNECTORS_CHART = ROOT / "src/backend/services/connectors/helm"
INSIGHT_CHART = ROOT / "charts/insight"


def render_connectors(chart: Path, *extra: str) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["helm", "template", "contract-test", str(chart), *extra],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


def connectors_documents(chart: Path, *extra: str) -> list[dict]:
    result = render_connectors(chart, *extra)
    assert result.returncode == 0, result.stderr
    return [doc for doc in yaml.safe_load_all(result.stdout) if doc]


@pytest.fixture(scope="session")
def connectors_umbrella() -> Path:
    result = subprocess.run(
        ["helm", "dependency", "build", str(INSIGHT_CHART), "--skip-refresh"],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    return INSIGHT_CHART
