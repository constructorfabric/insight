from __future__ import annotations

import concurrent.futures
import json
import os
import secrets
import shlex
import subprocess
import sys
import time
from collections.abc import Iterator, Mapping
from pathlib import Path
from unittest.mock import MagicMock

import delete_connector_schedule as deleter
import pymysql
import pytest
import sweep_connector_schedules as sweeper
from reconcile_inputs import Definition, Source, listing, plan_row
from schedule_database import connect
from test_connector_schedule_client import ENV
from test_schedule_account import create_schedule_schema

ROOT = Path(__file__).resolve().parents[1]


def prune(tmp_path: Path, env: Mapping[str, str], *, mode: str = "full") -> subprocess.CompletedProcess[str]:
    calls = tmp_path / "calls"
    sources = listing(
        [
            Source("example-main-example-tenant", "source-main", "definition"),
            Source("example-gone-example-tenant", "source-gone", "definition"),
            Source("other-main-example-tenant", "other-main", "other-definition"),
            Source("other-gone-example-tenant", "other-gone", "other-definition"),
        ]
    )
    definitions = listing([Definition("example", "definition"), Definition("other", "other-definition")])
    plan = plan_row("example", "main", "example-secret")
    if mode == "connector":
        plan += "\n" + plan_row("other", "main", "other-secret")
    script = f"""
    set -euo pipefail
    export INSIGHT_NAMESPACE=example CONNECTORS_DIR={shlex.quote(str(tmp_path))}
    export INSIGHT_TENANT_ID=example-tenant AIRBYTE_URL=http://127.0.0.1:1
    source {shlex.quote(str(ROOT / "lib/reconcile.sh"))}
    _RECONCILE_CHANGED=0
    _RECONCILE_FAILED=0
    RECONCILE_NO_GC={"1" if mode == "no-gc" else "0"}
    RECONCILE_DRY_RUN={"1" if mode == "dry-run" else "0"}
    reconcile__log() {{ printf '%s\\n' "$*"; }}
    disc_load_secrets() {{ printf 'example\tmain\texample-secret\thash\n'; }}
    ab_workspace_id() {{ printf workspace; }}
    ab_list_sources() {{ printf '%s' {shlex.quote(sources)}; }}
    ab_list_definitions() {{ printf '%s' {shlex.quote(definitions)}; }}
    ab_delete_source() {{ printf 'source %s\\n' "$1" >> {shlex.quote(str(calls))}; }}
    argo_delete_instance_cronworkflow() {{ printf 'job %s/%s\\n' "$1" "$3" >> {shlex.quote(str(calls))}; }}
    reconcile_prune_removed_instances {shlex.quote(plan)} {"example" if mode in {"connector", "source-id"} else "''"} {"main" if mode == "source-id" else "''"}
    reconcile_sweep_orphan_schedules {"example" if mode in {"connector", "source-id"} else "''"} {"main" if mode == "source-id" else "''"}
    printf 'counts %s %s\\n' "$_RECONCILE_CHANGED" "$_RECONCILE_FAILED"
    """
    return subprocess.run(
        ["bash", "-c", script],
        env={**env, "PATH": f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}"},
        text=True,
        capture_output=True,
        check=False,
        timeout=90,
    )


@pytest.mark.parametrize("missing", [False, True])
def test_deletion_is_scoped_and_uses_one_connection_under_the_shared_lock(database: MagicMock, missing: bool) -> None:
    database.fetchall.side_effect = [[(1,)], [(1,)]]
    database.rowcount = 0 if missing else 1
    deleter.run(ENV, "example", "gone")
    calls = database.execute.call_args_list
    assert [call.args for call in calls] == [
        ("SELECT GET_LOCK(SHA2(CONCAT(DATABASE(), '/', %s), 256), %s)", ("example/gone", 60)),
        ("DELETE FROM connector_schedules WHERE connector = %s AND source_id = %s", ("example", "gone")),
        ("SELECT RELEASE_LOCK(SHA2(CONCAT(DATABASE(), '/', %s), 256))", ("example/gone",)),
    ]
    pymysql.connect.assert_called_once()
    assert pymysql.connect.call_args.kwargs["read_timeout"] == 65
    assert pymysql.connect.call_args.kwargs["init_command"] == "SET SESSION max_statement_time=61"


@pytest.mark.parametrize("failure", ["locked", "unreachable", "delete"])
def test_failure_never_reports_a_success_and_releases_an_acquired_lock(
    database: MagicMock, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], failure: str
) -> None:
    for key, value in ENV.items():
        monkeypatch.setenv(key, value)
    monkeypatch.setattr(sys, "argv", ["delete_connector_schedule.py", "--connector", "example", "--source-id", "gone"])
    database.fetchall.side_effect = [[(0 if failure == "locked" else 1,)], [(1,)]]
    if failure == "unreachable":
        monkeypatch.setattr(pymysql, "connect", MagicMock(side_effect=RuntimeError("synthetic-password")))
    if failure == "delete":
        database.execute.side_effect = [None, RuntimeError("synthetic-password"), None]
    assert deleter.main() == 1
    error = capsys.readouterr().err
    assert "schedule delete failed:" in error
    assert "synthetic-password" not in error
    calls = database.execute.call_args_list
    if failure in {"locked", "unreachable"}:
        assert not any(call.args[0].startswith("DELETE") for call in calls)
    else:
        assert calls[-1].args[0].startswith("SELECT RELEASE_LOCK")


@pytest.mark.parametrize("connector", ["", "example"])
@pytest.mark.parametrize("failure", [False, True])
def test_sweep_covers_siblings_removed_connectors_and_recreated_rows(
    database: MagicMock,
    monkeypatch: pytest.MonkeyPatch,
    capsys: pytest.CaptureFixture[str],
    connector: str,
    failure: bool,
) -> None:
    rows = [("example", "main"), ("example", "gone"), ("other", "gone"), ("retired", "recreated")]
    database.fetchall.return_value = [row for row in rows if not connector or row[0] == connector]
    delete = MagicMock(side_effect=RuntimeError("synthetic failure") if failure else None)
    monkeypatch.setattr(sweeper, "delete", delete)
    expected = [("example", "gone")] if connector else rows[1:]
    secrets_tsv = "example\tmain\texample-secret\thash"
    for _ in range(2):
        assert sweeper.run(ENV, secrets_tsv, connector) == (len(expected) if failure else 0)
    assert [call.args[1:] for call in delete.call_args_list] == expected * 2
    query = database.execute.call_args.args
    assert ("WHERE connector = %s" in query[0]) == bool(connector)
    error = capsys.readouterr().err
    for name, source_id in expected:
        assert (f"schedule orphan delete failed for {name}/{source_id}: RuntimeError" in error) == failure


@pytest.mark.parametrize("descriptor", ["unshipped", "invalid"])
def test_secrets_keep_schedules_even_when_the_descriptor_cannot_be_used(
    database: MagicMock, monkeypatch: pytest.MonkeyPatch, tmp_path: Path, descriptor: str
) -> None:
    if descriptor == "invalid":
        (tmp_path / "descriptor.yaml").write_text("name: [\n")
    secret = {"metadata": {"name": "example-secret", "annotations": {"insight.cyberfabric.com/connector": "example"}}}
    script = f"""
    export INSIGHT_NAMESPACE=example CONNECTORS_DIR={shlex.quote(str(tmp_path))}
    source {shlex.quote(str(ROOT / "lib/discover.sh"))}
    kubectl() {{ printf '%s' {shlex.quote(json.dumps({"items": [secret]}))}; }}
    disc_load_instances >/dev/null || exit 1
    disc_load_secrets
    """
    result = subprocess.run(
        ["bash", "-c", script],
        env={**os.environ, "PATH": f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}"},
        text=True,
        capture_output=True,
        check=True,
    )
    database.fetchall.return_value = [("example", "main"), ("example", "gone"), ("other", "main")]
    delete = MagicMock()
    monkeypatch.setattr(sweeper, "delete", delete)
    assert sweeper.run(ENV, result.stdout) == 0
    assert [call.args[1:] for call in delete.call_args_list] == [("example", "gone"), ("other", "main")]


@pytest.mark.parametrize("failure", ["delete", "secrets"])
def test_orphan_sweep_failures_reach_the_structured_log_with_the_instance(tmp_path: Path, failure: str) -> None:
    helper = tmp_path / "sweep_connector_schedules.py"
    helper.write_text(
        "import sys\nprint('schedule orphan delete failed for example/gone: RuntimeError', file=sys.stderr)\nprint(1)\n"
    )
    secret_read = "return 1" if failure == "secrets" else "printf 'example\\tmain\\texample-secret\\thash\\n'"
    script = f"""
    set -euo pipefail
    export INSIGHT_NAMESPACE=example CONNECTORS_DIR={shlex.quote(str(tmp_path))}
    export AIRBYTE_URL=http://127.0.0.1:1
    source {shlex.quote(str(ROOT / "lib/reconcile.sh"))}
    _RECONCILE_PY_DIR={shlex.quote(str(tmp_path))}
    _RECONCILE_FAILED=0
    disc_load_secrets() {{ {secret_read}; }}
    reconcile_sweep_orphan_schedules
    [[ "$_RECONCILE_FAILED" -eq 1 ]]
    """
    result = subprocess.run(["bash", "-c", script], text=True, capture_output=True, check=False)
    assert result.returncode == 0, result.stdout + result.stderr
    logs = [json.loads(line) for line in result.stdout.splitlines()]
    assert len(logs) == 1, result.stdout + result.stderr
    assert logs[0]["level"] == "error"
    if failure == "delete":
        assert "example/gone" in logs[0]["msg"]
    else:
        assert "cannot read" in logs[0]["msg"]
    assert result.stderr == ""


@pytest.mark.parametrize("release_failure", ["answer", "exception"])
def test_release_failure_logs_and_preserves_the_completed_delete_answer(
    database: MagicMock, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], release_failure: str
) -> None:
    for key, value in ENV.items():
        monkeypatch.setenv(key, value)
    monkeypatch.setattr(sys, "argv", ["delete_connector_schedule.py", "--connector", "example", "--source-id", "gone"])
    database.fetchall.side_effect = [[(1,)], [(0,)]]
    if release_failure == "exception":
        database.execute.side_effect = [None, None, RuntimeError("synthetic-password")]
    assert deleter.main() == 0
    error = capsys.readouterr().err
    assert "schedule lock release failed:" in error
    assert "synthetic-password" not in error


@pytest.fixture
def live_schedules() -> Iterator[dict[str, str]]:
    if os.environ.get("SCHEDULE_GRANTS_LIVE") != "1":
        pytest.skip("requires disposable MariaDB")
    name = "schedule_test_" + secrets.token_hex(6)
    env = {**os.environ, "CONNECTOR_SCHEDULES_DB_NAME": name}
    env.pop("CONNECTOR_SCHEDULES_DB_NAME_FILE", None)
    with connect({**env, "CONNECTOR_SCHEDULES_DB_NAME": "mysql"}) as connection, connection.cursor() as cursor:
        cursor.execute(f"CREATE DATABASE `{name}`")
        try:
            cursor.execute(f"USE `{name}`")
            create_schedule_schema(cursor)
            for connector, source in [("example", "gone"), ("example", "main"), ("other", "gone")]:
                cursor.execute(
                    "INSERT INTO connector_schedules VALUES (%s, %s, '[\"0 3 * * *\"]', 8, UNHEX(REPEAT('00',16)), UTC_TIMESTAMP(3))",
                    (connector, source),
                )
            yield env
        finally:
            cursor.execute(f"DROP DATABASE `{name}`")


@pytest.mark.parametrize("mode", ["full", "connector", "source-id", "no-gc", "dry-run", "locked", "unreachable"])
def test_live_sweep_deletes_orphans_and_resource_removal_survives_database_failure(
    tmp_path: Path, live_schedules: dict[str, str], mode: str
) -> None:
    env = live_schedules
    with connect(env) as connection, connection.cursor() as cursor:
        if mode == "locked":
            cursor.execute("SELECT GET_LOCK(SHA2(CONCAT(DATABASE(), '/', 'example/gone'),256),0)")
            assert cursor.fetchall() == [(1,)]
        if mode == "unreachable":
            env = {**env, "CONNECTOR_SCHEDULES_DB_HOST": "127.0.0.1", "CONNECTOR_SCHEDULES_DB_PORT": "1"}
            env.pop("CONNECTOR_SCHEDULES_DB_HOST_FILE", None)
        started = time.monotonic()
        with concurrent.futures.ThreadPoolExecutor() as executor:
            pending = executor.submit(prune, tmp_path, env, mode=mode)
            if mode == "locked":
                time.sleep(0.5)
                assert not pending.done(), "held lock must make pruning wait"
                cursor.execute("SELECT COUNT(*) FROM connector_schedules")
                assert cursor.fetchall() == [(3,)]
            result = pending.result(timeout=85)
        assert result.returncode == 0, result.stdout + result.stderr
        failed = mode in {"locked", "unreachable"}
        active = mode in {"full", "connector", "locked", "unreachable"}
        assert f"counts {int(active)} {int(failed)}" in result.stdout
        if active:
            assert (tmp_path / "calls").read_text().splitlines() == ["source source-gone", "job example/gone"]
        else:
            assert not (tmp_path / "calls").exists()
        if mode == "locked":
            assert time.monotonic() - started >= 60
            errors = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
            assert any(
                entry["level"] == "error"
                and "example/gone" in entry["msg"]
                and "schedule instance lock is busy" in entry["msg"]
                for entry in errors
            ), result.stdout + result.stderr
            cursor.execute("SELECT RELEASE_LOCK(SHA2(CONCAT(DATABASE(), '/', 'example/gone'),256))")
            assert cursor.fetchall() == [(1,)]
        cursor.execute("SELECT connector, source_id FROM connector_schedules ORDER BY connector, source_id")
        remaining = list(cursor)
        expected = [("example", "gone"), ("example", "main"), ("other", "gone")]
        if mode in {"full", "connector", "locked"}:
            if mode != "connector":
                expected.remove(("other", "gone"))
            if mode != "locked":
                expected.remove(("example", "gone"))
        assert remaining == expected
        if failed:
            assert (tmp_path / "calls").exists()
            assert "schedule orphan" in result.stdout
            retry = prune(tmp_path, live_schedules)
            assert "counts 1 0" in retry.stdout, retry.stdout + retry.stderr
            cursor.execute("SELECT COUNT(*) FROM connector_schedules WHERE connector='example' AND source_id='gone'")
            assert cursor.fetchall() == [(0,)]


def test_live_sweep_retries_rows_recreated_after_resource_deletion(live_schedules: dict[str, str]) -> None:
    secrets_tsv = "example\tmain\texample-secret\thash"
    for _ in range(2):
        assert sweeper.run(live_schedules, secrets_tsv) == 0
        with connect(live_schedules) as connection, connection.cursor() as cursor:
            cursor.execute("SELECT connector, source_id FROM connector_schedules")
            assert cursor.fetchall() == [("example", "main")]
            cursor.execute(
                "INSERT INTO connector_schedules VALUES ('example', 'gone', '[\"0 3 * * *\"]', 1, UNHEX(REPEAT('00',16)), UTC_TIMESTAMP(3))"
            )


@pytest.mark.parametrize("descriptor", ["unshipped", "invalid"])
def test_live_sweep_preserves_secret_named_rows_without_a_usable_descriptor(
    tmp_path: Path, live_schedules: dict[str, str], descriptor: str
) -> None:
    if descriptor == "invalid":
        (tmp_path / "descriptor.yaml").write_text("name: [\n")
    secret = {"metadata": {"name": "example-secret", "annotations": {"insight.cyberfabric.com/connector": "example"}}}
    script = f"""
    set -euo pipefail
    export INSIGHT_NAMESPACE=example CONNECTORS_DIR={shlex.quote(str(tmp_path))}
    export AIRBYTE_URL=http://127.0.0.1:1
    source {shlex.quote(str(ROOT / "lib/reconcile.sh"))}
    kubectl() {{ printf '%s' {shlex.quote(json.dumps({"items": [secret]}))}; }}
    plan="$(disc_load_instances)"
    [[ -z "$plan" ]]
    _RECONCILE_FAILED=0
    reconcile_sweep_orphan_schedules
    [[ "$_RECONCILE_FAILED" -eq 0 ]]
    """
    result = subprocess.run(
        ["bash", "-c", script],
        env={**live_schedules, "PATH": f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}"},
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stdout + result.stderr
    with connect(live_schedules) as connection, connection.cursor() as cursor:
        cursor.execute("SELECT connector, source_id FROM connector_schedules")
        assert cursor.fetchall() == [("example", "main")]
