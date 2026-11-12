from __future__ import annotations

import base64
import json
import os
import subprocess
from unittest.mock import MagicMock

import provision_schedule_secret as secret
import pytest
from test_schedule_account import ACCOUNT_ENV

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
