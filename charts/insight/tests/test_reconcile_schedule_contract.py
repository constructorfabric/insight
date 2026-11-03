from __future__ import annotations

import subprocess
from pathlib import Path

import pytest
import yaml

CHART = Path(__file__).resolve().parents[1]


@pytest.mark.parametrize(("timezone", "expected"), [(None, "UTC"), ("Europe/London", "Europe/London"), ("", "")])
def test_connector_schedule_timezone_reaches_reconcile(timezone: str | None, expected: str) -> None:
    overrides = [] if timezone is None else ["--set-string", f"ingestion.reconcile.scheduleTimezone={timezone}"]
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
            "templates/ingestion/reconcile-cron.yaml",
        ],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    document = yaml.safe_load(result.stdout)
    env = document["spec"]["workflowSpec"]["templates"][0]["container"]["env"]
    assert next(item["value"] for item in env if item["name"] == "RECONCILE_SCHEDULE_TIMEZONE") == expected, (
        f"unexpected timezone for override: {timezone!r}"
    )
