from __future__ import annotations

import base64
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[1] / "python/provision_schedule_secret.py"
SUPPLIED = {"user": "example_supplied", "pass": "synthetic-supplied-password"}

KUBECTL = r"""
import base64
import json
import os
import sys
from pathlib import Path

state = Path(os.environ["FAKE_STATE"])
scenario = os.environ["FAKE_SCENARIO"]
args = sys.argv[1:]
with open(os.environ["FAKE_TRACE"], "a") as trace:
    trace.write(json.dumps({"argv": args}) + "\n")
assert all(not arg.startswith("--request-timeout") for arg in args), args
assert args[:2] == ["--namespace", "synthetic"]
if args[2] == "get":
    assert args[3:] == ["secret", "example-schedule-db", "--ignore-not-found=true", "-o", "json"]
    if scenario == "read-error":
        print("Forbidden: token=synthetic-token password=synthetic-supplied-password", file=sys.stderr)
        print("second-line-secret", file=sys.stderr)
        sys.exit(1)
    if state.exists():
        print(state.read_text())
    sys.exit(0)

assert args[2:] == ["create", "-f", "-"]
manifest = json.load(sys.stdin)
with open(os.environ["FAKE_TRACE"], "a") as trace:
    trace.write(json.dumps({"created": manifest}) + "\n")
data = manifest["stringData"]
if scenario == "race":
    data = {"user": "example_supplied", "pass": "synthetic-supplied-password"}
if scenario != "create-error":
    state.write_text(json.dumps({"data": {key: base64.b64encode(value.encode()).decode() for key, value in data.items()}}))
if scenario in {"race", "create-error"}:
    print("Conflict: " + json.dumps(manifest), file=sys.stderr)
    sys.exit(1)
"""

MARIADB = r"""
import json
import os

class Cursor:
    def __enter__(self):
        return self
    def __exit__(self, *args):
        pass
    def __iter__(self):
        return iter([])
    def execute(self, sql, params):
        with open(os.environ["FAKE_TRACE"], "a") as trace:
            trace.write(json.dumps({"sql": sql, "params": params}) + "\n")
        if os.environ["FAKE_SCENARIO"] == "database-error":
            raise RuntimeError(str(params))

class Connection:
    def __enter__(self):
        return self
    def __exit__(self, *args):
        pass
    def cursor(self):
        return Cursor()

class cursors:
    SSCursor = Cursor

def connect(**kwargs):
    return Connection()
"""


def hook(tmp_path: Path, scenario: str) -> tuple[subprocess.CompletedProcess[str], list[dict], Path]:
    binary = tmp_path / "kubectl"
    binary.write_text(f"#!{sys.executable}\n" + KUBECTL)
    binary.chmod(0o700)
    (tmp_path / "pymysql.py").write_text(MARIADB)
    state = tmp_path / "secret.json"
    if scenario in {"existing", "invalid", "empty"}:
        data = {key: base64.b64encode(value.encode()).decode() for key, value in SUPPLIED.items()}
        if scenario == "invalid":
            data["pass"] = "not base64!"
        if scenario == "empty":
            data["pass"] = ""
        state.write_text(json.dumps({"data": data, "metadata": {"labels": {"owner": "example"}}}))

    trace = tmp_path / "trace.jsonl"
    trace.write_text("")
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        env={
            **os.environ,
            "PATH": str(tmp_path) + os.pathsep + os.environ["PATH"],
            "PYTHONPATH": str(tmp_path),
            "FAKE_STATE": str(state),
            "FAKE_TRACE": str(trace),
            "FAKE_SCENARIO": scenario,
            "INSIGHT_NAMESPACE": "synthetic",
            "INSIGHT_RELEASE": "example-release",
            "CONNECTOR_SCHEDULES_SECRET_NAME": "example-schedule-db",
            "CONNECTOR_SCHEDULES_SECRET_USER_KEY": "user",
            "CONNECTOR_SCHEDULES_SECRET_PASSWORD_KEY": "pass",
            "CONNECTOR_SCHEDULES_ACCOUNT_USER": "example_schedule",
            "CONNECTOR_SCHEDULES_DB_HOST": "mariadb.example.test",
            "CONNECTOR_SCHEDULES_DB_NAME": "analytics",
            "CONNECTOR_SCHEDULES_DB_USER": "root",
            "CONNECTOR_SCHEDULES_DB_PASSWORD": "synthetic-root-password",
        },
        capture_output=True,
        text=True,
        timeout=30,
        check=False,
    )
    return result, [json.loads(line) for line in trace.read_text().splitlines()], state


@pytest.mark.parametrize("scenario", ["absent", "existing", "race"])
def test_runtime_secret_and_account_converge_without_exposing_passwords(tmp_path: Path, scenario: str) -> None:
    result, events, state = hook(tmp_path, scenario)
    assert result.returncode == 0, result.stderr
    assert result.stdout == result.stderr == ""
    created = [event["created"] for event in events if "created" in event]
    alterations = [event["params"] for event in events if event.get("sql", "").startswith("ALTER USER")]
    stored = {key: base64.b64decode(value).decode() for key, value in json.loads(state.read_text())["data"].items()}
    assert alterations[-1] == [stored["user"], stored["pass"]]
    assert len(alterations) == (2 if scenario == "race" else 1)
    assert len(created) == (0 if scenario == "existing" else 1)
    if scenario == "existing":
        assert json.loads(state.read_text())["metadata"] == {"labels": {"owner": "example"}}
    else:
        manifest = created[0]
        assert manifest["apiVersion"] == "v1" and manifest["kind"] == "Secret" and manifest["type"] == "Opaque"
        assert manifest["metadata"] == {
            "name": "example-schedule-db",
            "namespace": "synthetic",
            "labels": {
                "app.kubernetes.io/name": "insight",
                "app.kubernetes.io/instance": "example-release",
                "app.kubernetes.io/component": "connector-schedules",
                "app.kubernetes.io/managed-by": "schedule-db-provision",
            },
        }
        generated = manifest["stringData"]
        assert generated["user"] == "example_schedule"
        assert re.fullmatch(r"[A-Za-z0-9_-]{43,}", generated["pass"])
        assert alterations[0] == [generated["user"], generated["pass"]]
        create_index = next(i for i, event in enumerate(events) if "created" in event)
        assert any(event.get("sql", "").startswith("SET DEFAULT ROLE") for event in events[:create_index])

    if scenario in {"existing", "race"}:
        assert stored == SUPPLIED
    arguments = json.dumps([event["argv"] for event in events if "argv" in event])
    passwords = [params[1] for params in alterations] + ["synthetic-root-password"]
    for password in passwords:
        assert password not in arguments + result.stdout + result.stderr

    before = state.read_text()
    again, repeated, _ = hook(tmp_path, "reuse")
    assert again.returncode == 0, again.stderr
    assert state.read_text() == before
    assert not any("created" in event for event in repeated)
    assert [event["params"] for event in repeated if event.get("sql", "").startswith("ALTER USER")] == [
        [stored["user"], stored["pass"]]
    ]


@pytest.mark.parametrize(
    ("scenario", "cause"),
    [
        ("read-error", "secret read failed: CalledProcessError; kubectl exit code 1; Forbidden: [redacted]"),
        ("create-error", "secret create failed: CalledProcessError; kubectl exit code 1; Conflict: [redacted]"),
        ("invalid", "secret read failed: Error"),
        ("empty", "secret read failed: ValueError"),
        ("database-error", "account provisioning failed: RuntimeError"),
    ],
)
def test_runtime_secret_failures_are_closed_and_do_not_log_credentials(
    tmp_path: Path, scenario: str, cause: str
) -> None:
    result, events, state = hook(tmp_path, scenario)
    assert result.returncode == 1
    assert result.stdout == ""
    assert result.stderr == f"schedule {cause}\n"
    if scenario != "create-error":
        assert not any("created" in event for event in events)
    if scenario in {"read-error", "invalid", "empty"}:
        assert not any("sql" in event for event in events)
    if scenario in {"read-error", "create-error", "database-error"}:
        assert not state.exists()
