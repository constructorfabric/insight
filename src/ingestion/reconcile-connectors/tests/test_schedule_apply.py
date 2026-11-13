from __future__ import annotations

import concurrent.futures
import io
import json
import os
import secrets
import shlex
import sys
import time
from pathlib import Path
from unittest.mock import MagicMock

import apply_connector_schedule as applier
import pymysql
import pytest
import yaml
from schedule_database import connect
from test_connector_schedule_client import ENV
from test_cronworkflow_error_logging import run_caller
from test_render_cronworkflow import _render
from test_schedule_account import create_schedule_schema


@pytest.mark.parametrize("crons", [["0 8 * * *"], [], None])
def test_fresh_instance_read_and_apply_share_the_named_lock(
    database: MagicMock, monkeypatch: pytest.MonkeyPatch, crons: list[str] | None
) -> None:
    database.fetchall.side_effect = [
        [(1,)],
        [] if crons is None else [("example", "main", 8, json.dumps(crons))],
        [(1,)],
    ]
    events = []
    database.execute.side_effect = lambda sql, args: events.append(sql.split()[1])
    applied = []

    def apply(manifest: str) -> None:
        assert events == ["GET_LOCK(SHA2(CONCAT(DATABASE(),", "connector,"], events
        applied.append(yaml.safe_load(manifest))

    monkeypatch.setattr(applier, "apply", apply)
    manifest = _render("0 2 * * *").stdout
    applier.run(ENV, "example", "main", manifest)
    assert applied[0]["spec"]["schedules"] == (crons or ["0 2 * * *"])
    assert applied[0]["metadata"]["annotations"]["insight.cyberfabric.com/schedule-origin"] == (
        "api" if crons else "default"
    )
    options = pymysql.connect.call_args.kwargs
    assert options["read_timeout"] == 65
    assert options["init_command"] == "SET SESSION max_statement_time=61"
    database.fetchone.assert_not_called()
    calls = database.execute.call_args_list
    assert calls[0].args == ("SELECT GET_LOCK(SHA2(CONCAT(DATABASE(), '/', %s), 256), %s)", ("example/main", 60))
    assert calls[1].args[1] == ("example", "main")
    assert "WHERE connector = %s AND source_id = %s" in calls[1].args[0]
    assert calls[-1].args == ("SELECT RELEASE_LOCK(SHA2(CONCAT(DATABASE(), '/', %s), 256))", ("example/main",))


def test_lock_timeout_never_reads_or_applies(database: MagicMock, monkeypatch: pytest.MonkeyPatch) -> None:
    database.fetchall.return_value = [(0,)]
    apply = MagicMock()
    monkeypatch.setattr(applier, "apply", apply)
    with pytest.raises(applier.ScheduleReadError, match="lock is busy"):
        applier.run(ENV, "example", "main", "unused")
    database.execute.assert_called_once()
    database.fetchall.assert_called_once()
    apply.assert_not_called()


@pytest.mark.parametrize("failure", ["read", "apply"])
def test_failures_release_the_lock_without_changing_a_schedule(
    database: MagicMock, monkeypatch: pytest.MonkeyPatch, failure: str
) -> None:
    database.fetchall.side_effect = [[(1,)], [("example", "main", 8, '["0 8 * * *"]')], [(1,)]]
    if failure == "read":
        database.fetchall.side_effect = [[(1,)], RuntimeError("synthetic read failure"), [(1,)]]
    apply = MagicMock(side_effect=RuntimeError("synthetic apply failure"))
    monkeypatch.setattr(applier, "apply", apply)
    with pytest.raises(RuntimeError, match="synthetic"):
        applier.run(ENV, "example", "main", _render("0 2 * * *").stdout)
    assert database.execute.call_args.args[0].startswith("SELECT RELEASE_LOCK")
    assert apply.call_count == (1 if failure == "apply" else 0)


def test_unreachable_database_never_applies(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(pymysql, "connect", MagicMock(side_effect=RuntimeError("synthetic outage")))
    apply = MagicMock()
    monkeypatch.setattr(applier, "apply", apply)
    with pytest.raises(RuntimeError):
        applier.run(ENV, "example", "main", "unused")
    apply.assert_not_called()


def test_release_failure_keeps_the_successful_apply_answer(
    database: MagicMock, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    database.fetchall.side_effect = [[(1,)], [], [(0,)]]
    apply = MagicMock()
    monkeypatch.setattr(applier, "apply", apply)
    monkeypatch.setattr(sys, "stdin", io.StringIO(_render("0 2 * * *").stdout))
    monkeypatch.setattr(sys, "argv", ["apply_connector_schedule.py", "--connector", "example", "--source-id", "main"])
    for key, value in ENV.items():
        monkeypatch.setenv(key, value)
    assert applier.main() == 0
    apply.assert_called_once()
    assert "schedule lock release failed" in capsys.readouterr().err


@pytest.mark.skipif(os.environ.get("SCHEDULE_GRANTS_LIVE") != "1", reason="requires disposable MariaDB")
@pytest.mark.parametrize("mode", ["locked", "changed", "clear", "unreachable"])
def test_pass_waits_then_fails_or_applies_the_fresh_row(tmp_path: Path, mode: str) -> None:
    database_name = "schedule_test_" + secrets.token_hex(6)
    env = {**os.environ, "CONNECTOR_SCHEDULES_DB_NAME": database_name}
    output = tmp_path / "applied.yaml"
    binaries = tmp_path / "bin"
    binaries.mkdir()
    kubectl = binaries / "kubectl"
    kubectl.write_text(f"#!/bin/sh\ncat > {shlex.quote(str(output))}\n")
    kubectl.chmod(0o755)
    with connect({**env, "CONNECTOR_SCHEDULES_DB_NAME": "mysql"}) as connection, connection.cursor() as cursor:
        cursor.execute(f"CREATE DATABASE `{database_name}`")
        try:
            cursor.execute(f"USE `{database_name}`")
            create_schedule_schema(cursor)
            cursor.execute(
                "INSERT INTO connector_schedules VALUES ('example-connector', 'main', '[\"0 3 * * *\"]', 1, UNHEX(REPEAT('00',16)), UTC_TIMESTAMP(3))"
            )
            lock_sql = "SELECT GET_LOCK(SHA2(CONCAT(DATABASE(), '/', 'example-connector/main'),256),0)"
            cursor.execute(lock_sql)
            assert cursor.fetchone() == (1,)
            assignments = "\n".join(
                f"export {name}={shlex.quote(value)}"
                for name, value in env.items()
                if name.startswith("CONNECTOR_SCHEDULES_DB_")
            )
            overrides = f"""
            {assignments}
            export PATH={shlex.quote(str(binaries))}:"$PATH"
            argo_render_cronworkflow() {{ cat {shlex.quote(str(tmp_path / "base.yaml"))}; }}
            _argo_superseded_names() {{ :; }}
            argo_delete_former_tenant_cronworkflows() {{ :; }}
            connector_schedules_read
            """
            (tmp_path / "base.yaml").write_text(_render("0 2 * * *").stdout)
            if mode == "locked":
                started = time.monotonic()
                result = run_caller("reconcile", tmp_path, overrides)
                assert time.monotonic() - started >= 60
                assert result.returncode == 1, result.stdout + result.stderr
                assert "schedule instance lock is busy" in result.stdout
                assert not output.exists()
            else:
                cursor.execute("SELECT RELEASE_LOCK(SHA2(CONCAT(DATABASE(), '/', 'example-connector/main'),256))")
                barrier = tmp_path / "snapshot-read"
                resume = tmp_path / "resume"
                overrides += f"""
                reconcile_connections() {{
                  touch {shlex.quote(str(barrier))}
                  while [[ ! -f {shlex.quote(str(resume))} ]]; do sleep 0.05; done
                  printf 'noop\\texample-connection\\n'
                }}
                """
                if mode == "unreachable":
                    overrides += "export CONNECTOR_SCHEDULES_DB_HOST=127.0.0.1 CONNECTOR_SCHEDULES_DB_PORT=1\n"
                    overrides += "unset CONNECTOR_SCHEDULES_DB_HOST_FILE\n"
                with concurrent.futures.ThreadPoolExecutor() as executor:
                    pending = executor.submit(run_caller, "reconcile", tmp_path, overrides)
                    deadline = time.monotonic() + 10
                    while not barrier.exists() and time.monotonic() < deadline:
                        time.sleep(0.05)
                    assert barrier.exists(), "pass did not reach the snapshot barrier"
                    cursor.execute(
                        "UPDATE connector_schedules SET crons=%s, revision=2",
                        ("[]" if mode == "clear" else '["0 8 * * *"]',),
                    )
                    resume.touch()
                    result = pending.result(timeout=15)
                assert result.returncode == (1 if mode == "unreachable" else 0), result.stdout + result.stderr
                if mode == "unreachable":
                    assert not output.exists()
                else:
                    assert yaml.safe_load(output.read_text())["spec"]["schedules"] == (
                        ["0 2 * * *"] if mode == "clear" else ["0 8 * * *"]
                    )
        finally:
            cursor.execute(f"DROP DATABASE `{database_name}`")


@pytest.mark.parametrize("mode", ["changed", "clear", "locked", "unreachable"])
def test_instance_apply_uses_the_lock_client_and_never_the_pass_snapshot(tmp_path: Path, mode: str) -> None:
    modules = tmp_path / "modules"
    modules.mkdir()
    journal = tmp_path / "queries"
    crons = [] if mode == "clear" else ["0 8 * * *"]
    (modules / "pymysql.py").write_text(
        "from pathlib import Path\nfrom unittest.mock import MagicMock\n"
        "cursors = MagicMock()\n"
        "def connect(**options):\n"
        + ("    raise RuntimeError('synthetic outage')\n" if mode == "unreachable" else "")
        + "    connection = MagicMock()\n"
        "    connection.__enter__.return_value = connection\n"
        "    cursor = connection.cursor.return_value.__enter__.return_value\n"
        f"    cursor.fetchall.side_effect = [[{(0,) if mode == 'locked' else (1,)!r}], [('example-connector', 'main', 8, {json.dumps(crons)!r})], [(1,)]]\n"
        "    def record(sql, parameters):\n"
        f"        with Path({str(journal)!r}).open('a') as out: out.write(sql + '\\n')\n"
        "    cursor.execute.side_effect = record\n"
        "    return connection\n"
    )
    applied = tmp_path / "applied.yaml"
    binaries = tmp_path / "bin"
    binaries.mkdir()
    kubectl = binaries / "kubectl"
    kubectl.write_text(f"#!/bin/sh\ncat > {shlex.quote(str(applied))}\n")
    kubectl.chmod(0o755)
    deployment = tmp_path / "deployment.yaml"
    deployment.write_text(_render("0 2 * * *").stdout)
    prepared = tmp_path / "prepared"
    overrides = f"""
    export CONNECTOR_SCHEDULES_DB_HOST=example-db CONNECTOR_SCHEDULES_DB_NAME=connectors
    export CONNECTOR_SCHEDULES_DB_USER=example_reader CONNECTOR_SCHEDULES_DB_PASSWORD=synthetic-password
    export PYTHONPATH={shlex.quote(str(modules))}:"${{PYTHONPATH-}}"
    export PATH={shlex.quote(str(binaries))}:"$PATH"
    reconcile_resolve_schedule() {{ printf 'api\\n0 3 * * *'; }}
    reconcile_resolve_deployment_schedule() {{ printf 'default\\n0 2 * * *'; }}
    argo_render_cronworkflow() {{
      printf '%s|%s' "$3" "$7" > {shlex.quote(str(prepared))}
      cat {shlex.quote(str(deployment))}
    }}
    _argo_superseded_names() {{ :; }}
    argo_delete_former_tenant_cronworkflows() {{ :; }}
    """
    result = run_caller("reconcile", tmp_path, overrides)
    assert prepared.read_text() == "0 2 * * *|default"
    if mode in {"locked", "unreachable"}:
        assert result.returncode == 1, result.stdout + result.stderr
        assert not applied.exists()
        assert "failed to create/update Argo CronWorkflow" in result.stdout
    else:
        assert result.returncode == 0, result.stdout + result.stderr
        assert yaml.safe_load(applied.read_text())["spec"]["schedules"] == (crons or ["0 2 * * *"])
        queries = journal.read_text().splitlines()
        assert queries[0].startswith("SELECT GET_LOCK")
        assert "WHERE connector = %s AND source_id = %s" in queries[1]
        assert queries[2].startswith("SELECT RELEASE_LOCK")
