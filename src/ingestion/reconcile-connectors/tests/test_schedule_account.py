from __future__ import annotations

import os
import re
import secrets
from pathlib import Path
from unittest.mock import MagicMock

import provision_schedule_account as account
import pymysql
import pytest
from acknowledge_connector_schedule import acknowledge
from schedule_database import connect
from test_connector_schedule_client import ENV
from test_schedule_acknowledgements import OBSERVED, PLAN

ACCOUNT_ENV = {
    **ENV,
    "CONNECTOR_SCHEDULES_ACCOUNT_USER": "example_schedule",
    "CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD": "synthetic-account-password",
}


def test_provisioning_always_runs(database: MagicMock, monkeypatch: pytest.MonkeyPatch) -> None:
    for key, value in ACCOUNT_ENV.items():
        monkeypatch.setenv(key, value)
    database.__iter__.side_effect = lambda: iter([])
    assert account.main() == 0
    database.execute.assert_any_call(
        "ALTER USER %s@'%%' IDENTIFIED BY %s", ("example_schedule", "synthetic-account-password")
    )


def test_provisioning_failure_names_its_stage_and_exception_class(
    database: MagicMock, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    for key, value in ACCOUNT_ENV.items():
        monkeypatch.setenv(key, value)
    database.execute.side_effect = RuntimeError("synthetic-account-password")

    assert account.main() == 1
    captured = capsys.readouterr()
    assert captured.out == ""
    assert captured.err == "schedule account provisioning failed: RuntimeError\n"


def test_provisioning_grants_only_the_versioned_view_and_acknowledgement_access(database: MagicMock) -> None:
    database.__iter__.side_effect = lambda: iter([])
    account.provision(ACCOUNT_ENV)
    calls = [call.args for call in database.execute.call_args_list]
    grants = [sql for sql, _ in calls if sql.startswith("GRANT")]
    assert grants == [
        "GRANT SELECT ON `analytics`.connector_schedule_requests_v1 TO `example_schedule_role`",
        "GRANT SELECT, INSERT, UPDATE ON `analytics`.connector_schedule_acknowledgements TO `example_schedule_role`",
        "GRANT `example_schedule_role` TO %s@'%%'",
    ]
    assert calls[0] == (
        "CREATE USER IF NOT EXISTS %s@'%%' IDENTIFIED BY %s",
        ("example_schedule", "synthetic-account-password"),
    )
    assert calls[1] == ("ALTER USER %s@'%%' IDENTIFIED BY %s", ("example_schedule", "synthetic-account-password"))
    assert calls[-1] == ("SET DEFAULT ROLE `example_schedule_role` FOR %s@'%%'", ("example_schedule",))
    assert all("synthetic-account-password" not in sql for sql, _ in calls)


@pytest.mark.parametrize("key", ["CONNECTOR_SCHEDULES_ACCOUNT_USER", "CONNECTOR_SCHEDULES_DB_NAME"])
def test_provisioning_rejects_sql_identifiers(database: MagicMock, key: str) -> None:
    with pytest.raises(ValueError):
        account.provision({**ACCOUNT_ENV, key: "bad`identifier"})
    database.execute.assert_not_called()


@pytest.mark.skipif(
    os.environ.get("SCHEDULE_GRANTS_LIVE") != "1",
    reason="set SCHEDULE_GRANTS_LIVE=1 for a disposable MariaDB grant test",
)
def test_live_schedule_account_has_only_its_four_object_privileges() -> None:
    suffix = secrets.token_hex(6)
    database = "schedule_test_" + suffix
    user = "schedule_test_" + suffix
    role = user + "_role"
    admin_env = dict(os.environ)
    admin_env["CONNECTOR_SCHEDULES_DB_NAME"] = "mysql"
    account_env = {
        **admin_env,
        "CONNECTOR_SCHEDULES_DB_NAME": database,
        "CONNECTOR_SCHEDULES_ACCOUNT_USER": user,
        "CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD": secrets.token_hex(24),
    }
    runtime = {
        **account_env,
        "CONNECTOR_SCHEDULES_DB_USER": user,
        "CONNECTOR_SCHEDULES_DB_PASSWORD": account_env["CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD"],
    }
    runtime.pop("CONNECTOR_SCHEDULES_DB_USER_FILE", None)
    runtime.pop("CONNECTOR_SCHEDULES_DB_PASSWORD_FILE", None)
    migration = (
        Path(__file__).resolve().parents[3]
        / "backend/services/analytics/src/migration/m20261103_000001_connector_schedules.rs"
    )
    with connect(admin_env) as admin, admin.cursor() as cursor:
        try:
            cursor.execute(f"CREATE DATABASE `{database}`", ())
            cursor.execute(f"USE `{database}`", ())
            statements = re.findall(r'execute_unprepared\(\s*"(.*?)"', migration.read_text(), re.DOTALL)
            assert statements, migration
            for statement in statements:
                cursor.execute(statement, ())
            account.provision(account_env)
            account.provision(account_env)
            with connect(runtime) as connection, connection.cursor() as narrow:
                narrow.execute("SELECT connector, source_id, revision, crons FROM connector_schedule_requests_v1", ())
                assert list(narrow) == []
            assert acknowledge(runtime, {"plan": PLAN, "observed": OBSERVED})
            assert acknowledge(runtime, {"plan": PLAN, "observed": OBSERVED})
            assert not acknowledge(runtime, {"plan": PLAN, "error": "schedule apply failed"})
            cursor.execute(
                "SELECT applied_revision, attempted_revision, last_error FROM connector_schedule_acknowledgements", ()
            )
            assert list(cursor) == [(7, 7, "schedule apply failed")]
            with connect(runtime) as connection, connection.cursor() as narrow:
                narrow.execute(
                    "SELECT applied_revision, attempted_revision, last_error FROM connector_schedule_acknowledgements",
                    (),
                )
                assert list(narrow) == [(7, 7, "schedule apply failed")]
            forbidden = [
                "SELECT * FROM connector_schedules",
                "DELETE FROM connector_schedule_acknowledgements",
                "DELETE FROM connector_schedule_requests_v1",
                "UPDATE connector_schedules SET revision=2",
                "CREATE TABLE forbidden (id INT)",
                "ALTER TABLE connector_schedule_acknowledgements ADD COLUMN forbidden INT",
                "DROP TABLE connector_schedule_acknowledgements",
                "SELECT * FROM mysql.user",
            ]
            with connect(runtime) as connection, connection.cursor() as narrow:
                for sql in forbidden:
                    try:
                        narrow.execute(sql, ())
                    except pymysql.MySQLError as exc:
                        assert exc.args[0] in {1044, 1142, 1143}, sql
                    else:
                        pytest.fail(f"forbidden privilege accepted: {sql}")
        finally:
            cursor.execute("DROP USER IF EXISTS %s@'%%'", (user,))
            cursor.execute(f"DROP ROLE IF EXISTS `{role}`", ())
            cursor.execute(f"DROP DATABASE IF EXISTS `{database}`", ())
