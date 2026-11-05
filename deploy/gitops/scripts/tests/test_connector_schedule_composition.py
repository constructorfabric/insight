from __future__ import annotations

import json
import shlex
import subprocess
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[4]
SCRIPT = ROOT / "deploy/gitops/scripts/compose-app-secrets.sh"
KEY = "APP__gears__analytics__config__connector_schedules__reconcile_schedules"


def compose_fragment(tmp_path: Path, schedule: str | None) -> subprocess.CompletedProcess[str]:
    values: dict = {}
    if schedule is not None:
        values["ingestion"] = {"reconcile": {"schedule": schedule}}
    path = tmp_path / "values.yaml"
    path.write_text(yaml.safe_dump(values))
    source = SCRIPT.read_text()
    start = source.index("CONNECTOR_RECONCILE_SCHEDULES=$(")
    config = source[start : source.index("# ── L2 connection coordinates")]
    start = source.index('  CONNECTOR_RECONCILE_SCHEDULES="$CONNECTOR_RECONCILE_SCHEDULES" yq')
    emit = source[start : source.index("} | kubectl", start)]
    assignments = f"ROOT={shlex.quote(str(SCRIPT.parents[1]))}\nVALUES={shlex.quote(str(path))}"
    return subprocess.run(
        ["/bin/bash", "-c", f"set -euo pipefail\n{assignments}\n{config}\n{emit}"],
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


@pytest.mark.parametrize("schedule", [None, "7 */2 * * *"])
def test_composition_uses_the_same_default_or_override_as_the_chart(tmp_path: Path, schedule: str | None) -> None:
    result = compose_fragment(tmp_path, schedule)
    assert result.returncode == 0, result.stderr
    default = yaml.safe_load((ROOT / "charts/insight/values.yaml").read_text())["ingestion"]["reconcile"]["schedule"]
    assert json.loads(yaml.safe_load(result.stdout)[KEY]) == [schedule if schedule is not None else default]


def test_composition_refuses_an_empty_cadence(tmp_path: Path) -> None:
    result = compose_fragment(tmp_path, "")
    assert result.returncode != 0
    assert "set ingestion.reconcile.schedule" in result.stderr
