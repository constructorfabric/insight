from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

CHART = Path(__file__).resolve().parents[1]


@pytest.fixture(scope="session")
def chart_dependencies() -> None:
    result = subprocess.run(
        ["helm", "dependency", "update", str(CHART)], capture_output=True, text=True, timeout=300, check=False
    )
    assert result.returncode == 0, result.stderr


@pytest.mark.parametrize("threshold", [None, 3600])
def test_poller_bounds_unreadability_without_a_job_deadline(chart_dependencies: None, threshold: int | None) -> None:
    overrides = (
        [] if threshold is None else ["--set", f"ingestion.airbyteSync.statusUnreadableThresholdSeconds={threshold}"]
    )
    result = subprocess.run(
        [
            "helm",
            "template",
            "contract-test",
            str(CHART),
            "--values",
            str(CHART / "tests" / "values.yaml"),
            *overrides,
            "--show-only",
            "templates/ingestion/airbyte-sync.yaml",
        ],
        capture_output=True,
        text=True,
        timeout=300,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    out = result.stdout
    assert "activeDeadlineSeconds:" not in out
    assert "IDLE_THRESHOLD_SECONDS" not in out
    assert f'- name: STATUS_UNREADABLE_THRESHOLD_SECONDS\n            value: "{threshold or 1800}"' in out
