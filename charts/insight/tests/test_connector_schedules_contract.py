from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]
CADENCE_KEY = "APP__gears__analytics__config__connector_schedules__reconcile_schedules"


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


def resource(documents: list[dict], kind: str, name: str) -> dict:
    return next(doc for doc in documents if doc and doc["kind"] == kind and doc["metadata"]["name"] == name)


def test_reported_reconcile_cadence_is_the_cronworkflow_cadence() -> None:
    result = render("--set-string", "ingestion.reconcile.schedule=7 */2 * * *")
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    cron = resource(documents, "CronWorkflow", "contract-test-reconcile-loop")
    secret = resource(documents, "Secret", "insight-analytics-config")
    assert json.loads(secret["stringData"][CADENCE_KEY]) == cron["spec"]["schedules"] == ["7 */2 * * *"]


@pytest.mark.parametrize("gitops", [False, True])
def test_schedule_requests_require_a_reconcile_cadence(gitops: bool) -> None:
    overrides = ["--set", "credentials.autoGenerate=false"] if gitops else []
    result = render("--set-string", "ingestion.reconcile.schedule=", *overrides)
    assert result.returncode != 0
    assert "ingestion.reconcile.schedule is required" in result.stderr
