from __future__ import annotations

import base64
import json
import os
import shlex
import subprocess
import sys
from pathlib import Path
from unittest.mock import MagicMock

import provision_schedule_secret as secret
import pytest
from test_schedule_account import ACCOUNT_ENV

ROOT = Path(__file__).resolve().parents[1]
SECRET_ENV = {
    **ACCOUNT_ENV,
    "INSIGHT_NAMESPACE": "example-namespace",
    "INSIGHT_RELEASE": "example-release",
    "CONNECTOR_SCHEDULES_SECRET_NAME": "example-schedule-db",
    "CONNECTOR_SCHEDULES_SECRET_USER_KEY": "user",
    "CONNECTOR_SCHEDULES_SECRET_PASSWORD_KEY": "pass",
}


@pytest.mark.parametrize("operation", ["get", "create"])
@pytest.mark.parametrize("failure", ["timeout", "missing-binary"])
def test_secret_subprocess_failures_name_the_stage_without_exception_contents(
    operation: str,
    failure: str,
    monkeypatch: pytest.MonkeyPatch,
    database: MagicMock,
    capsys: pytest.CaptureFixture[str],
) -> None:
    monkeypatch.setattr(os, "environ", SECRET_ENV)
    database.__iter__.side_effect = lambda: iter([])
    calls = []
    exception = (
        subprocess.TimeoutExpired(["synthetic-token"], 45, stderr=b"synthetic-password")
        if failure == "timeout"
        else FileNotFoundError("synthetic-password")
    )

    def run(argv: list[str], **kwargs: object) -> subprocess.CompletedProcess[str]:
        calls.append(argv)
        assert not any(arg.startswith("--request-timeout") for arg in argv), argv
        assert kwargs["timeout"] == 45
        if argv[3] == operation:
            raise exception
        return subprocess.CompletedProcess(argv, 0, "", "")

    monkeypatch.setattr(subprocess, "run", run)
    assert secret.main() == 1
    stage = "secret read" if operation == "get" else "secret create"
    assert capsys.readouterr().err == f"schedule {stage} failed: {type(exception).__name__}\n"
    assert len(calls) == (1 if operation == "get" else 2)


@pytest.mark.parametrize(
    ("stderr", "safe"),
    [
        ("Forbidden: access denied\nsecond-line-secret", "Forbidden: access denied"),
        ("Unauthorized: Bearer synthetic-token", "Unauthorized: [redacted]"),
        ("Forbidden: password=short", "Forbidden: [redacted]"),
        ("Forbidden: api key = short", "Forbidden: [redacted]"),
        ("Invalid: c3ludGhldGlj", "Invalid: [redacted]"),
        ('Invalid: {"data":{"pass":"c3ludGhldGlj"}}', "Invalid: [redacted]"),
        ('Invalid: "synthetic-supplied-password"', "Invalid: [redacted]"),
        ("Failed: https://example:synthetic-password@example.test", "Failed: [redacted]"),
        ("synthetic-supplied-password", "[redacted]"),
        ("", "<empty stderr>"),
    ],
)
def test_secret_read_errors_keep_only_a_safe_first_stderr_line(
    stderr: str, safe: str, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr(os, "environ", SECRET_ENV)
    process = subprocess.CompletedProcess(["kubectl"], 7, "secret-data-must-not-be-printed", stderr)
    monkeypatch.setattr(subprocess, "run", MagicMock(return_value=process))

    assert secret.main() == 1
    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == f"schedule secret read failed: CalledProcessError; kubectl exit code 7; {safe}\n"


@pytest.mark.parametrize(
    ("output", "stderr", "exit_code", "cause"),
    [
        ('{"metadata":{},"spec":{}}', "", 0, ""),
        (
            "",
            "Forbidden: token=synthetic-token\nsecond-line-secret",
            7,
            "CalledProcessError; kubectl exit code 7; Forbidden: [redacted]",
        ),
        ("synthetic-secret-data", "", 0, "JSONDecodeError"),
        ("[]", "", 0, "ValueError"),
    ],
)
def test_readback_uses_in_cluster_argv_and_reports_safe_stage_failures(
    tmp_path: Path, output: str, stderr: str, exit_code: int, cause: str
) -> None:
    trace = tmp_path / "argv.json"
    binary = tmp_path / "kubectl"
    binary.write_text(
        f"#!{sys.executable}\nimport json, sys\nfrom pathlib import Path\n"
        f"Path({str(trace)!r}).write_text(json.dumps(sys.argv[1:]))\n"
        f"sys.stdout.write({output!r})\nsys.stderr.write({stderr!r})\nraise SystemExit({exit_code})\n"
    )
    binary.chmod(0o700)
    client = tmp_path / "client"
    client.mkdir()
    (client / "acknowledge_connector_schedule.py").write_text("import sys\nsys.stdout.write(sys.stdin.read())\n")
    script = f"""
        set -euo pipefail
        source {shlex.quote(str(ROOT / "lib/connector-schedules.sh"))}
        _CONNECTOR_SCHEDULES_PY_DIR={shlex.quote(str(client))}
        _CONNECTOR_SCHEDULES_STATUS=ok
        _CONNECTOR_SCHEDULES_RESULT='{{"schedules":[]}}'
        argo_cron_workflow_name() {{ printf 'example-main-sync'; }}
        connector_schedules_acknowledge example main tenant '0 1 * * *' api $'default\\n0 2 * * *' 0
    """
    result = subprocess.run(
        ["bash", "-c", script],
        env={**os.environ, "INSIGHT_NAMESPACE": "example-namespace", "PATH": f"{tmp_path}:{os.environ['PATH']}"},
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    argv = json.loads(trace.read_text())
    assert not any(arg.startswith("--request-timeout") for arg in argv), argv
    assert argv == ["-n", "example-namespace", "get", "cronworkflow.argoproj.io/example-main-sync", "-o", "json"]
    payload = json.loads(result.stdout)
    assert payload["error"] == ("schedule readback failed" if cause else None)
    assert payload["observed"] == ({} if cause else {"metadata": {}, "spec": {}})
    assert result.stderr == (f"schedule cronworkflow readback failed: {cause}\n" if cause else "")


def test_existing_account_failure_names_provisioning_without_logging_secret_data(
    monkeypatch: pytest.MonkeyPatch, database: MagicMock, capsys: pytest.CaptureFixture[str]
) -> None:
    monkeypatch.setattr(os, "environ", SECRET_ENV)
    data = {
        key: base64.b64encode(value.encode()).decode() for key, value in {"user": "example", "pass": "short"}.items()
    }
    process = subprocess.CompletedProcess(["kubectl"], 0, json.dumps({"data": data}), "")
    monkeypatch.setattr(subprocess, "run", MagicMock(return_value=process))
    database.execute.side_effect = RuntimeError("short synthetic-root-password")

    assert secret.main() == 1
    assert capsys.readouterr().err == "schedule account provisioning failed: RuntimeError\n"
