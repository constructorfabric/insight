from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from copy import deepcopy
from pathlib import Path
from unittest.mock import MagicMock

import pytest
import read_schedule_cronworkflow as reader
import schedule_apply as loop
from test_connector_schedule_client import ENV
from test_connector_schedule_runs import ROOT, bash
from test_schedule_acknowledgements import OBSERVED, PLAN, inserted


@pytest.mark.parametrize("operation", ["patch", "get"])
@pytest.mark.parametrize("failure", ["exit", "timeout", "missing-binary", "json"])
def test_loop_failures_name_the_stage_and_keep_only_safe_causes(
    operation: str,
    failure: str,
    database: MagicMock,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
) -> None:
    observed = deepcopy(OBSERVED)
    observed["metadata"].update(uid="example-uid", resourceVersion="12")
    if operation == "patch":
        observed["spec"]["schedules"] = ["0 8 * * *"]
    exceptions = {
        "exit": subprocess.CalledProcessError(
            7, ["synthetic-token"], output="synthetic-password", stderr="Forbidden: token=short\nprivate-data"
        ),
        "timeout": subprocess.TimeoutExpired(["synthetic-token"], 20, stderr=b"synthetic-password"),
        "missing-binary": FileNotFoundError("synthetic-password"),
    }

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        assert command[3] == operation
        if failure in exceptions:
            raise exceptions[failure]
        return subprocess.CompletedProcess(command, 0, "synthetic-password", "")

    monkeypatch.setattr(subprocess, "run", run)
    assert not loop.apply_instance(ENV, deepcopy(PLAN), observed, {})
    stage = "cronworkflow patch" if operation == "patch" else "cronworkflow readback"
    cause = {
        "exit": "CalledProcessError; kubectl exit code 7; Forbidden: [redacted]",
        "timeout": "TimeoutExpired",
        "missing-binary": "FileNotFoundError",
        "json": "JSONDecodeError",
    }[failure]
    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == f"schedule {stage} failed: {cause}\n"
    row, _ = inserted(database)
    assert row["last_error"] == ("schedule apply failed" if operation == "patch" else "schedule readback failed")


@pytest.mark.parametrize("failure", [False, True])
def test_loop_listing_uses_in_cluster_argv_and_safe_stage_diagnostics(tmp_path: Path, failure: bool) -> None:
    trace = tmp_path / "argv.json"
    binary = tmp_path / "kubectl"
    binary.write_text(
        f"#!{sys.executable}\nimport json, sys\nfrom pathlib import Path\n"
        f"Path({str(trace)!r}).write_text(json.dumps(sys.argv[1:]))\n"
        f"sys.stderr.write({'Forbidden: password=short' if failure else ''!r})\n"
        f"print('{{\"items\":[]}}')\nraise SystemExit({7 if failure else 0})\n"
    )
    binary.chmod(0o700)
    result = bash(
        f"""
        set -euo pipefail
        source {shlex.quote(str(ROOT / "schedule-apply.sh"))}
        export PATH={shlex.quote(str(tmp_path))}:"${{PATH}}"
        connector_schedules_read() {{ :; }}
        connector_schedules_writable() {{ return 0; }}
        disc_load_instances() {{ :; }}
        python3() {{
          case "$1" in
            */read_schedule_descriptor.py|*/schedule_apply.py) return 0 ;;
            *) command python3 "$@" ;;
          esac
        }}
        schedule_apply {shlex.quote(str(tmp_path))}
        """,
        tmp_path,
    )
    argv = json.loads(trace.read_text())
    assert not any(arg.startswith("--request-timeout") for arg in argv), argv
    assert argv == [
        "-n",
        "example-namespace",
        "get",
        "cronworkflows.argoproj.io",
        "-l",
        "app.kubernetes.io/component=connector-sync",
        "-o",
        "json",
    ]
    assert result.returncode == int(failure), result.stderr
    assert result.stdout == ""
    assert result.stderr == (
        "schedule cronworkflow list failed: CalledProcessError; kubectl exit code 7; Forbidden: [redacted]\n"
        if failure
        else ""
    )


@pytest.mark.parametrize("failure", ["timeout", "missing-binary", "json", "shape"])
def test_loop_listing_is_bounded_and_reports_safe_non_exit_failures(
    failure: str, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr(sys, "argv", ["read_schedule_cronworkflow.py", "example-namespace", "--list"])

    def run(command: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        assert kwargs["timeout"] == 15
        if failure == "timeout":
            raise subprocess.TimeoutExpired(command, 15, stderr=b"synthetic-password")
        if failure == "missing-binary":
            raise FileNotFoundError("synthetic-password")
        return subprocess.CompletedProcess(command, 0, "synthetic-password" if failure == "json" else "[]", "")

    monkeypatch.setattr(subprocess, "run", run)
    assert reader.main() == 1
    cause = {
        "timeout": "TimeoutExpired",
        "missing-binary": "FileNotFoundError",
        "json": "JSONDecodeError",
        "shape": "ValueError",
    }[failure]
    assert capsys.readouterr().err == f"schedule cronworkflow list failed: {cause}\n"


def test_application_entrypoint_reports_safe_failure_cause(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    plans = tmp_path / "plans.jsonl"
    plans.write_text("synthetic-password")
    monkeypatch.setattr(sys, "argv", ["schedule_apply.py", str(plans), "unused"])
    monkeypatch.setattr(os, "environ", ENV)
    assert loop.main() == 1
    assert capsys.readouterr().err == "schedule application failed: JSONDecodeError\n"
