from __future__ import annotations

import json
import logging
import os
import shlex
import subprocess
import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python"))

from sweep.inherited import warn_shadowed_changes
from sweep.plan import Instance, plan_snapshot
from sweep.schedule import AppliedSchedule, read_schedule

ROOT = Path(__file__).resolve().parents[1]
INSTANCE = Instance("example-connector", "example-tenant", "main")


@pytest.mark.parametrize("requested", [True, False])
@pytest.mark.parametrize(
    ("annotation", "descriptor", "origin", "crons"),
    [
        ("0 1 * * *\n0 13 * * *", "schedule: '0 2 * * *'", "annotation", ["0 1 * * *", "0 13 * * *"]),
        ("", "schedule:\n  - '0 2 * * *'\n  - '0 14 * * *'", "descriptor", ["0 2 * * *", "0 14 * * *"]),
        ("", "", "default", ["0 0 * * *"]),
    ],
)
def test_inherited_schedule_ignores_the_request_and_is_recorded_per_instance(
    tmp_path: Path, requested: bool, annotation: str, descriptor: str, origin: str, crons: list[str]
) -> None:
    directory = tmp_path / "example-area" / "example-connector"
    directory.mkdir(parents=True)
    (directory / "descriptor.yaml").write_text(f"name: example-connector\n{descriptor}\n")
    script = f"""
    set -euo pipefail
    source "{ROOT}/lib/connector-naming.sh"
    source "{ROOT}/lib/sweep.sh"
    kubectl() {{
      if [[ "$*" == *cronworkflows.argoproj.io* ]]; then
        printf '%s' '{{"items":[]}}'
      else
        printf '%s' {shlex.quote(annotation)}
      fi
    }}
    log_line() {{ :; }}
    reconcile_compute_tenant() {{ printf 'example-tenant'; }}
    disc_load_instances() {{ printf '%s\\n' {shlex.quote(chr(9).join(["example-connector", str(directory), "1", "type", "image", "selector", "bronze_example", "main", "example-secret", "hash"]))}; }}
    reconcile_resolve_schedule example-connector example-secret main
    printf '\\n---\\n'
    sweep__build_work tick-example '[]'
    """
    requests = [{"connector": "example-connector", "source_id": "main", "crons": ["0 6 * * *"]}] if requested else []
    result = subprocess.run(
        ["bash", "-c", script],
        env={
            **os.environ,
            "INSIGHT_NAMESPACE": "example-namespace",
            "CONNECTORS_DIR": str(tmp_path),
            "_CONNECTOR_SCHEDULES_STATUS": "ok",
            "_CONNECTOR_SCHEDULES_RESULT": json.dumps({"schedules": requests}),
            "PATH": f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}",
        },
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    applied, work = result.stdout.split("\n---\n")
    assert applied.splitlines() == (["api", "0 6 * * *"] if requested else [origin, *crons])
    connector = json.loads(work)["connectors"][0]
    assert connector["schedule_requested"] is requested
    row = plan_snapshot([INSTANCE], "tick-example", {INSTANCE: read_schedule(connector)})[0]
    assert row["inherited_schedules"] == crons
    assert row["inherited_schedule_origin"] == origin


@pytest.mark.parametrize(
    ("requested", "previous", "origin", "expected"),
    [
        (True, (("0 1 * * *",), "annotation"), "annotation", 1),
        (True, (("0 1 * * *",), "descriptor"), "descriptor", 1),
        (True, (("0 2 * * *",), "annotation"), "annotation", 0),
        (True, (("0 2 * * *",), "annotation"), "descriptor", 0),
        (False, (("0 1 * * *",), "annotation"), "annotation", 0),
        (True, ((), ""), "annotation", 0),
        (True, None, "annotation", 0),
    ],
)
def test_only_a_shadowed_cron_change_warns_once(
    caplog: pytest.LogCaptureFixture,
    requested: bool,
    previous: tuple[tuple[str, ...], str] | None,
    origin: str,
    expected: int,
) -> None:
    schedule = AppliedSchedule(("0 6 * * *",), "UTC", "api", False, ("0 2 * * *",), origin, requested)
    with caplog.at_level(logging.WARNING, logger="sweep"):
        warn_shadowed_changes({INSTANCE: schedule}, {} if previous is None else {INSTANCE: previous})
        warn_shadowed_changes({INSTANCE: schedule}, {INSTANCE: (schedule.inherited_schedules, origin)})
    assert len(caplog.records) == expected
    if expected:
        for value in ["example-connector/example-tenant/main", "0 1 * * *", "0 2 * * *", "overridden by API request"]:
            assert value in caplog.text
