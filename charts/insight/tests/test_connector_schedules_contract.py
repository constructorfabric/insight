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


def enabled(*overrides: str) -> subprocess.CompletedProcess[str]:
    return render("--set", "global.connectorSchedules.enabled=true", *overrides)


def resource(documents: list[dict], kind: str, name: str) -> dict:
    return next(doc for doc in documents if doc and doc["kind"] == kind and doc["metadata"]["name"] == name)


@pytest.mark.parametrize("gitops", [False, True])
def test_one_switch_enables_the_api(gitops: bool) -> None:
    overrides = ["--set", "credentials.autoGenerate=false"] if gitops else []
    result = enabled(*overrides)
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    config = resource(documents, "ConfigMap", "contract-test-analytics-gears-config")
    gear = yaml.safe_load(config["data"]["analytics.yaml"])
    assert gear["gears"]["analytics"]["config"]["connector_schedules"]["enabled"] is True


def test_reported_reconcile_cadence_is_the_cronworkflow_cadence() -> None:
    result = enabled("--set-string", "ingestion.reconcile.schedule=7 */2 * * *")
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    cron = resource(documents, "CronWorkflow", "contract-test-reconcile-loop")
    secret = resource(documents, "Secret", "insight-analytics-config")
    assert json.loads(secret["stringData"][CADENCE_KEY]) == cron["spec"]["schedules"] == ["7 */2 * * *"]


def test_the_default_switch_disables_api_writes() -> None:
    result = render()
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    gear = resource(documents, "ConfigMap", "contract-test-analytics-gears-config")["data"]["analytics.yaml"]
    assert yaml.safe_load(gear)["gears"]["analytics"]["config"]["connector_schedules"]["enabled"] is False


def test_enabled_switch_requires_a_reconcile_cadence() -> None:
    result = enabled("--set-string", "ingestion.reconcile.schedule=")
    assert result.returncode != 0
    assert "ingestion.reconcile.schedule is required" in result.stderr


@pytest.mark.parametrize("minimum", [None, 60])
def test_schedule_minimum_is_rendered_in_the_analytics_config(minimum: int | None) -> None:
    overrides = [] if minimum is None else ["--set", f"analytics.connectorSchedules.minIntervalMinutes={minimum}"]
    result = render(*overrides)
    assert result.returncode == 0, result.stderr
    documents = list(yaml.safe_load_all(result.stdout))
    config = resource(documents, "ConfigMap", "contract-test-analytics-gears-config")
    gear = yaml.safe_load(config["data"]["analytics.yaml"])
    assert gear["gears"]["analytics"]["config"]["connector_schedules"]["min_interval_minutes"] == (
        30 if minimum is None else minimum
    )
