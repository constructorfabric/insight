from __future__ import annotations

import json
import shlex
import subprocess
import sys
from copy import deepcopy
from pathlib import Path
from unittest.mock import MagicMock

import pytest
import schedule_apply as loop
from test_connector_schedule_client import ENV
from test_connector_schedule_runs import ROOT, bash
from test_schedule_acknowledgements import OBSERVED, PLAN, inserted


@pytest.mark.parametrize(
    "case", ["drift", "unchanged", "new-revision", "missing", "patch-failed", "readback-failed", "mismatch", "replaced"]
)
def test_application_is_fenced_verified_and_acknowledges_its_snapshot(
    case: str, database: MagicMock, monkeypatch: pytest.MonkeyPatch
) -> None:
    observed = deepcopy(OBSERVED)
    observed["metadata"].update(uid="example-uid", resourceVersion="12")
    if case in {"drift", "patch-failed", "readback-failed", "mismatch", "replaced"}:
        observed["spec"]["schedules"] = ["0 8 * * *"]
    record = {
        "connector": PLAN["connector"],
        "source_id": PLAN["source_id"],
        "applied_revision": 6 if case == "new-revision" else 7,
        "applied_crons": json.dumps(PLAN["crons"]),
        "applied_timezone": "UTC",
        "applied_origin": "api",
        "suspended": True,
        "deployment_crons": json.dumps(PLAN["deployment_crons"]),
        "deployment_origin": PLAN["deployment_origin"],
        "last_error": None,
        "observed_at": "2026-01-01",
    }
    database.__iter__.side_effect = lambda: iter([tuple(record[column] for column in loop.ACK_COLUMNS)])
    calls = []

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        assert command[0] == "kubectl", command
        assert not any(arg.startswith("--request-timeout") for arg in command), command
        assert kwargs["timeout"] == 20
        assert command[3] in {"patch", "get"} and command[4] == "cronworkflow.argoproj.io/" + PLAN["name"]
        if command[3] == "patch":
            if case == "patch-failed":
                raise subprocess.CalledProcessError(1, command)
            patch = json.loads(command[command.index("-p") + 1])
            assert patch[:2] == [
                {"op": "test", "path": "/metadata/uid", "value": "example-uid"},
                {"op": "test", "path": "/metadata/resourceVersion", "value": "12"},
            ]
            assert patch[2:] == [
                {"op": "add", "path": "/spec/schedules", "value": PLAN["crons"]},
                {"op": "add", "path": "/spec/timezone", "value": "UTC"},
                {"op": "add", "path": "/metadata/annotations/insight.cyberfabric.com~1schedule-origin", "value": "api"},
            ]
            observed["spec"]["schedules"] = PLAN["crons"]
        elif case == "readback-failed":
            raise subprocess.CalledProcessError(1, command)
        elif case == "mismatch":
            observed["spec"]["timezone"] = "Europe/Sofia"
        elif case == "replaced":
            observed["metadata"]["uid"] = "replacement-uid"
        return subprocess.CompletedProcess(command, 0, json.dumps(observed))

    monkeypatch.setattr(subprocess, "run", run)
    workflows = {"items": [] if case == "missing" else [deepcopy(observed)]}
    succeeded = loop.apply_schedules(ENV, [deepcopy(PLAN)], workflows)
    assert succeeded == (case in {"drift", "unchanged", "new-revision"}), case
    assert not any("airbyte" in " ".join(call).lower() for call in calls)
    assert not any("create" in call or "submit" in call for call in calls)
    patches = [call for call in calls if call[3] == "patch"]
    assert len(patches) == int(case in {"drift", "patch-failed", "readback-failed", "mismatch", "replaced"}), case
    if case == "unchanged":
        assert not calls
        assert database.execute.call_count == 1
        return
    row, updates = inserted(database)
    assert row["attempted_revision"] == 7
    if succeeded:
        assert row["applied_revision"] == 7
        return
    expected = {
        "missing": "next full pass creates the missing CronWorkflow",
        "patch-failed": "schedule apply failed",
        "readback-failed": "schedule readback failed",
        "mismatch": "schedule readback mismatch",
        "replaced": "schedule readback mismatch",
    }
    assert row["last_error"] == expected[case]
    assert "applied_revision" not in updates and "observed_at" not in updates


@pytest.mark.parametrize("failure", ["database", "identity", "collision", "write"])
def test_unreadable_inputs_change_nothing_and_write_failures_fail_the_tick(
    failure: str, database: MagicMock, monkeypatch: pytest.MonkeyPatch
) -> None:
    observed = deepcopy(OBSERVED)
    observed["metadata"].update(uid="example-uid", resourceVersion="12")
    plans = [deepcopy(PLAN)]
    if failure == "identity":
        observed["metadata"]["labels"]["insight.cyberfabric.com/tenant"] = "other"
    if failure == "collision":
        plans.append(deepcopy(PLAN))
    database.__iter__.side_effect = lambda: iter([])
    database.execute.side_effect = RuntimeError("synthetic database failure") if failure == "database" else None
    calls = []

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(command)
        assert failure == "write" and command[3] == "get"
        database.execute.side_effect = RuntimeError("synthetic write failure")
        return subprocess.CompletedProcess(command, 0, json.dumps(observed))

    monkeypatch.setattr(subprocess, "run", run)
    with pytest.raises((RuntimeError, ValueError)):
        loop.apply_schedules(ENV, plans, {"items": [observed]})
    if failure != "write":
        assert not calls
        assert all(call.args[0].startswith("SELECT ") for call in database.execute.call_args_list)


@pytest.mark.parametrize(
    "failure", ["none", "database", "secrets", "annotation", "annotation-later", "descriptor", "workflows"]
)
def test_entrypoint_reads_every_input_before_starting_application(tmp_path: Path, failure: str) -> None:
    directory = tmp_path / "area/example"
    directory.mkdir(parents=True)
    descriptor = directory / "descriptor.yaml"
    descriptor.write_text("name: example\nversion: 1\nschedule: '0 1 * * *'\n")
    if failure == "descriptor":
        descriptor.write_text("invalid: [")
    client = tmp_path / "client"
    client.mkdir()
    snapshot = {"status": "ok", "schedules": [{**PLAN, "revision": 7}]}
    if failure == "database":
        snapshot = {"status": "unavailable", "reason": "synthetic failure"}
    (client / "read_connector_schedules.py").write_text(f"print({json.dumps(snapshot)!r})\n")
    plan = f"example\t{directory}\t1\tnocode\t\t\t\tmain\texample-secret\thash"
    if failure == "annotation-later":
        plan += f"\nexample\t{directory}\t1\tnocode\t\t\t\tsecond\tsecond-secret\thash"
    calls = tmp_path / "calls"
    calls.touch()
    binary = tmp_path / "kubectl"
    binary.write_text(
        f"#!{sys.executable}\nimport sys\n"
        "args = sys.argv[1:]\n"
        "assert not any(arg.startswith('--request-timeout') for arg in args), args\n"
        "if 'cronworkflows.argoproj.io' in args:\n"
        f"    if {failure == 'workflows'}: raise SystemExit(1)\n"
        "    print('{\"items\":[]}')\n"
        "elif 'example-secret' in args:\n"
        f"    if {failure == 'annotation'}: raise SystemExit(1)\n"
        "    print('0 2 * * *')\n"
        "else:\n    raise SystemExit(1)\n"
    )
    binary.chmod(0o700)
    result = bash(
        f"""
        set -euo pipefail
        source '{ROOT}/schedule-apply.sh'
        _CONNECTOR_SCHEDULES_PY_DIR='{client}'
        disc_load_instances() {{
          [[ '{failure}' != secrets ]] || return 1
          printf '%s' {shlex.quote(plan)}
        }}
        export PATH='{tmp_path}':"${{PATH}}"
        curl() {{ printf 'airbyte-call\n' >> '{calls}'; return 1; }}
        python3() {{
          if [[ "$1" == *'/schedule_apply.py' ]]; then
            printf 'apply\n' >> '{calls}'
            cat "$2"
          else
            command python3 "$@"
          fi
        }}
        schedule_apply '{tmp_path}'
        """,
        tmp_path,
    )
    assert (result.returncode == 0) == (failure == "none"), result.stdout + result.stderr
    assert calls.read_text().splitlines() == (["apply"] if failure == "none" else [])
    if failure == "none":
        rendered = json.loads(result.stdout)
        assert rendered["revision"] == 7
        assert rendered["crons"] == PLAN["crons"]
        assert rendered["deployment_crons"] == ["0 2 * * *"]
