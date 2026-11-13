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

operation = args[2]
assert args[2:] in (["create", "-f", "-"], ["patch", "secret", "example-schedule-db", "--type=merge", "--patch-file=/dev/stdin"])
manifest = json.load(sys.stdin)
with open(os.environ["FAKE_TRACE"], "a") as trace:
    trace.write(json.dumps({"created" if operation == "create" else "patched": manifest}) + "\n")
data = manifest["stringData"]
if scenario == "race":
    data = {"user": "example_supplied", "pass": "synthetic-supplied-password"}
if scenario == "patch-race":
    data = {"user": os.environ["CONNECTOR_SCHEDULES_ACCOUNT_USER"], "pass": "synthetic-winner-password"}
if operation == "patch":
    previous = json.loads(state.read_text())
    assert manifest["metadata"]["resourceVersion"] == previous["metadata"]["resourceVersion"]
else:
    previous = {"metadata": manifest["metadata"]}
if scenario == "race":
    previous["metadata"] = {"labels": {"owner": "example"}}
if scenario not in {"create-error", "patch-error"}:
    previous["data"] = {key: base64.b64encode(value.encode()).decode() for key, value in data.items()}
    previous["metadata"]["resourceVersion"] = "2"
    state.write_text(json.dumps(previous))
if scenario in {"race", "create-error", "patch-race", "patch-error"}:
    print("Conflict: " + json.dumps(manifest), file=sys.stderr)
    sys.exit(1)
"""

MARIADB = r"""
import json
import os
from pathlib import Path

class MySQLError(Exception):
    pass

state_path = Path(os.environ["FAKE_DB_STATE"])

def read():
    return json.loads(state_path.read_text()) if state_path.exists() else {"users": {}, "roles": {}}

def write(state):
    state_path.write_text(json.dumps(state))

class Cursor:
    def __init__(self, user):
        self.user = user
        self.rows = []
    def __enter__(self):
        return self
    def __exit__(self, *args):
        pass
    def __iter__(self):
        return iter(self.rows)
    def execute(self, sql, params=None):
        formatted_sql = sql % params if params is not None else sql
        with open(os.environ["FAKE_TRACE"], "a") as trace:
            trace.write(json.dumps({"sql": sql, "params": params, "formatted_sql": formatted_sql}) + "\n")
        if os.environ["FAKE_SCENARIO"] == "database-error":
            raise RuntimeError(str(params))
        state = read()
        users, roles = state["users"], state["roles"]
        self.rows = []
        if sql.startswith("SELECT is_role"):
            name, host = params
            self.rows = ([("N",)] if name in users else []) if host == "%" else ([("Y",)] if name in roles else [])
        elif sql.startswith("SHOW GRANTS"):
            name = sql.split("`")[1]
            entries = roles[name]["grants"] if "@" not in sql else users[name]["grants"]
            self.rows = [(grant,) for grant in entries]
        elif sql.startswith("SELECT User, Host"):
            role = roles.get(params[0], {})
            members = role.get("members", [])
            if "CURRENT_USER()" in sql:
                members = members + [admin for admin in role.get("admins", []) if admin != [self.user, "%"]]
            self.rows = [tuple(member) for member in members]
        elif sql == "SELECT CURRENT_USER()":
            self.rows = [(self.user + "@%",)]
        elif sql.startswith("CREATE USER"):
            name, password = params
            assert name not in users
            users[name] = {"password": password, "grants": [f"GRANT USAGE ON *.* TO `{name}`@`%` IDENTIFIED BY PASSWORD '*ABCD'"]}
        elif sql.startswith("CREATE ROLE"):
            role = sql.split("`")[1]
            assert role not in roles
            roles[role] = {"grants": [f"GRANT USAGE ON *.* TO `{role}`"], "members": [], "admins": [[self.user, "%"]]}
        elif sql.startswith("GRANT SELECT"):
            table = "connector_schedules" if "connector_schedules TO" in sql else "connector_schedule_requests_v1"
            privileges = "SELECT (`connector`, `source_id`), DELETE" if table == "connector_schedules" else "SELECT"
            database, role = sql.split("`")[1::2]
            grant = f"GRANT {privileges} ON `{database}`.`{table}` TO `{role}`"
            if grant not in roles[role]["grants"]:
                roles[role]["grants"].append(grant)
        elif sql.startswith(("GRANT `", "SET DEFAULT ROLE")):
            role = sql.split("`")[1]
            user = params[0]
            member = [user, "%"]
            if member not in roles[role]["members"]:
                roles[role]["members"].append(member)
            grant = f"GRANT `{role}` TO `{user}`@`%`" if sql.startswith("GRANT") else f"SET DEFAULT ROLE `{role}` FOR `{user}`@`%`"
            if grant not in users[user]["grants"]:
                users[user]["grants"].append(grant)
        else:
            raise AssertionError(sql)
        write(state)

class Connection:
    def __init__(self, user):
        self.user = user
    def __enter__(self):
        return self
    def __exit__(self, *args):
        pass
    def cursor(self):
        return Cursor(self.user)

class cursors:
    SSCursor = Cursor

def connect(**kwargs):
    user = kwargs["user"]
    if user != "root" and read()["users"].get(user, {}).get("password") != kwargs["password"]:
        raise MySQLError(1045, "synthetic authentication refusal")
    return Connection(user)
"""


def hook(
    tmp_path: Path, scenario: str, *, overrides: dict[str, str] | None = None
) -> tuple[subprocess.CompletedProcess[str], list[dict], Path]:
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
            "FAKE_DB_STATE": str(tmp_path / "accounts.json"),
            "INSIGHT_NAMESPACE": "synthetic",
            "INSIGHT_RELEASE": "example-release",
            "CONNECTOR_SCHEDULES_SECRET_NAME": "example-schedule-db",
            "CONNECTOR_SCHEDULES_SECRET_USER_KEY": "user",
            "CONNECTOR_SCHEDULES_SECRET_PASSWORD_KEY": "pass",
            "CONNECTOR_SCHEDULES_ACCOUNT_USER": "example_schedule",
            "CONNECTOR_SCHEDULES_DB_HOST": "mariadb.example.test",
            "CONNECTOR_SCHEDULES_DB_NAME": "connectors",
            "CONNECTOR_SCHEDULES_DB_USER": "root",
            "CONNECTOR_SCHEDULES_DB_PASSWORD": "synthetic-root-password",
            **(overrides or {}),
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
    creations = [event["params"] for event in events if event.get("sql", "").startswith("CREATE USER")]
    stored = {key: base64.b64decode(value).decode() for key, value in json.loads(state.read_text())["data"].items()}
    assert creations == [[stored["user"], stored["pass"]]]
    assert not any(event.get("sql", "").startswith(("ALTER", "REVOKE")) for event in events)
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
        if scenario != "race":
            assert creations[0] == [generated["user"], generated["pass"]]
        create_index = next(i for i, event in enumerate(events) if "created" in event)
        assert not any("sql" in event for event in events[:create_index])

    if scenario in {"existing", "race"}:
        assert stored == SUPPLIED
    arguments = json.dumps([event["argv"] for event in events if "argv" in event])
    passwords = [params[1] for params in creations] + ["synthetic-root-password"]
    for password in passwords:
        assert password not in arguments + result.stdout + result.stderr

    before = state.read_text()
    again, repeated, _ = hook(tmp_path, "reuse")
    assert again.returncode == 0, again.stderr
    assert state.read_text() == before
    assert not any("created" in event for event in repeated)
    assert not any(event.get("sql", "").startswith(("CREATE", "ALTER", "REVOKE")) for event in repeated)


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
    if scenario not in {"create-error", "database-error"}:
        assert not any("created" in event for event in events)
    if scenario in {"read-error", "invalid", "empty"}:
        assert not any("sql" in event for event in events)
    if scenario in {"read-error", "create-error"}:
        assert not state.exists()
    if scenario == "database-error":
        assert state.exists()
