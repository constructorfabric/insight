from __future__ import annotations

import base64
import json
import os
import secrets
from pathlib import Path

import provision_schedule_account as account
import pymysql
import pytest
from schedule_database import connect
from schedule_failure import AccountConflict
from test_schedule_account import create_schedule_schema
from test_schedule_secret import hook


@pytest.mark.parametrize("database", ["example_one", "a" * 63, "a" * 63 + "b", "a" * 63 + "c"])
def test_default_account_and_role_names_are_stable_and_bounded(database: str) -> None:
    env = {"CONNECTOR_SCHEDULES_DB_NAME": database}
    user = account.account_name(env)
    assert user == account.account_name(env)
    assert user != account.account_name({"CONNECTOR_SCHEDULES_DB_NAME": "example_two"})
    assert len(user) <= 80
    assert user.startswith("insight_schedule_")
    role = account.bounded_name(user + "_role")
    assert len(role) <= 80
    assert role != user


@pytest.mark.parametrize("user", ["example_override", "u" * 80])
def test_explicit_account_names_win_and_role_suffixes_stay_bounded(user: str) -> None:
    assert (
        account.account_name({"CONNECTOR_SCHEDULES_ACCOUNT_USER": user, "CONNECTOR_SCHEDULES_DB_NAME": "example"})
        == user
    )
    role = account.bounded_name(user + "_role")
    assert len(role) <= 80
    assert role != account.bounded_name(user + "_other_role")


@pytest.mark.parametrize(
    ("grant", "for_role"),
    [
        ("GRANT SELECT ON `other`.* TO `example_role`", True),
        ("GRANT SELECT ON `example`.`connector_schedules` TO `example_role`", True),
        ("GRANT SELECT ON `example`.`connector_schedule_requests_v1` TO `example_role` WITH GRANT OPTION", True),
        ("GRANT ALL PRIVILEGES ON *.* TO `example`@`%`", False),
        ("GRANT `other_role` TO `example`@`%`", False),
        ("GRANT `example_role` TO `example`@`%` WITH ADMIN OPTION", False),
        ("SET DEFAULT ROLE `other_role` FOR `example`@`%`", False),
    ],
)
def test_incompatible_grants_are_refused(grant: str, for_role: bool) -> None:
    with pytest.raises(AccountConflict):
        account.check_grants([grant], "example", "example_role", "example", for_role=for_role)


@pytest.mark.parametrize("columns", [None, "`connector`, `source_id`", "`source_id`, `connector`"])
def test_exact_role_grants_are_accepted_and_extra_write_privileges_are_refused(columns: str | None) -> None:
    grants = ["GRANT SELECT ON `example`.`connector_schedule_requests_v1` TO `example_role`"]
    if columns is not None:
        grants.append(f"GRANT SELECT ({columns}), DELETE ON `example`.`connector_schedules` TO `example_role`")
    account.check_grants(grants, "example", "example_role", "example", for_role=True)
    grants.append("GRANT INSERT ON `example`.`connector_schedules` TO `example_role`")
    with pytest.raises(AccountConflict):
        account.check_grants(grants, "example", "example_role", "example", for_role=True)


def test_deploy_adds_delete_privileges_to_an_existing_view_only_role(tmp_path: Path) -> None:
    result, _, _ = hook(tmp_path, "absent")
    assert result.returncode == 0, result.stderr
    database = tmp_path / "accounts.json"
    state = json.loads(database.read_text())
    role = state["roles"]["example_schedule_role"]
    role["grants"] = [grant for grant in role["grants"] if "connector_schedules`" not in grant]
    database.write_text(json.dumps(state))
    result, _, _ = hook(tmp_path, "reuse")
    assert result.returncode == 0, result.stderr
    upgraded = json.loads(database.read_text())
    assert upgraded["users"] == state["users"]
    assert upgraded["roles"]["example_schedule_role"]["grants"] == role["grants"] + [
        "GRANT SELECT (`connector`, `source_id`), DELETE ON `connectors`.`connector_schedules` TO `example_schedule_role`"
    ]


def test_two_installations_keep_separate_accounts_on_the_same_server(tmp_path: Path) -> None:
    shared = tmp_path / "accounts.json"
    installations = [tmp_path / name for name in ["one", "two"]]
    for directory in installations:
        directory.mkdir()
    snapshots = []
    for directory, database in zip(installations, ["example_one", "example_two"], strict=True):
        result, _, _ = hook(
            directory,
            "absent",
            overrides={
                "FAKE_DB_STATE": str(shared),
                "CONNECTOR_SCHEDULES_DB_NAME": database,
                "CONNECTOR_SCHEDULES_ACCOUNT_USER": "",
            },
        )
        assert result.returncode == 0, result.stderr
        snapshots.append(json.loads(shared.read_text()))

    first, both = snapshots
    assert set(both["users"]) == {"insight_schedule_example_one", "insight_schedule_example_two"}
    assert set(both["roles"]) == {"insight_schedule_example_one_role", "insight_schedule_example_two_role"}
    for kind in ["users", "roles"]:
        for name, value in first[kind].items():
            assert both[kind][name] == value
    result, _, _ = hook(
        installations[0],
        "reuse",
        overrides={
            "FAKE_DB_STATE": str(shared),
            "CONNECTOR_SCHEDULES_DB_NAME": "example_one",
            "CONNECTOR_SCHEDULES_ACCOUNT_USER": "",
        },
    )
    assert result.returncode == 0, result.stderr
    assert json.loads(shared.read_text()) == both


@pytest.mark.parametrize("conflict", ["user-grant", "role-grant", "role-member", "role-admin", "password"])
def test_conflicting_accounts_are_refused_without_database_or_secret_changes(tmp_path: Path, conflict: str) -> None:
    result, _, secret = hook(tmp_path, "absent")
    assert result.returncode == 0, result.stderr
    database = tmp_path / "accounts.json"
    state = json.loads(database.read_text())
    if conflict == "user-grant":
        state["users"]["example_schedule"]["grants"].append("GRANT SELECT ON `other`.* TO `example_schedule`@`%`")
    elif conflict == "role-grant":
        state["roles"]["example_schedule_role"]["grants"].append("GRANT SELECT ON `other`.* TO `example_schedule_role`")
    elif conflict == "role-member":
        state["roles"]["example_schedule_role"]["members"].append(["other", "%"])
    elif conflict == "role-admin":
        state["roles"]["example_schedule_role"]["admins"].append(["other", "%"])
    else:
        state["users"]["example_schedule"]["password"] = "synthetic-other-password"
    database.write_text(json.dumps(state))
    secret_before = secret.read_text()

    result, events, _ = hook(tmp_path, "reuse")

    assert result.returncode == 1
    assert "account or role already exists with incompatible grants or credentials" in result.stderr
    assert json.loads(database.read_text()) == state
    assert secret.read_text() == secret_before
    assert not any(event.get("sql", "").startswith(("CREATE", "ALTER", "REVOKE", "GRANT", "SET")) for event in events)


@pytest.mark.parametrize("scenario", ["reuse", "patch-race"])
def test_generated_secret_upgrade_changes_only_its_credentials_and_leaves_the_old_account(
    tmp_path: Path, scenario: str
) -> None:
    old_account = {
        "password": "synthetic-old-password",
        "grants": ["GRANT SELECT ON `other`.* TO `insight_schedule`@`%`"],
    }
    accounts = tmp_path / "accounts.json"
    accounts.write_text(json.dumps({"users": {"insight_schedule": old_account}, "roles": {}}))
    labels = {
        "app.kubernetes.io/managed-by": "schedule-db-provision",
        "app.kubernetes.io/instance": "example-release",
        "app.kubernetes.io/component": "connector-schedules",
        "example-label": "preserved",
    }
    secret = tmp_path / "secret.json"
    secret.write_text(
        json.dumps(
            {
                "metadata": {"labels": labels, "resourceVersion": "1"},
                "data": {
                    key: base64.b64encode(value.encode()).decode()
                    for key, value in {"user": "insight_schedule", "pass": old_account["password"]}.items()
                },
            }
        )
    )
    target = "insight_schedule_example"
    result, events, _ = hook(tmp_path, scenario, overrides={"CONNECTOR_SCHEDULES_ACCOUNT_USER": target})

    assert result.returncode == 0, result.stderr
    stored = json.loads(secret.read_text())
    credentials = {key: base64.b64decode(value).decode() for key, value in stored["data"].items()}
    assert credentials["user"] == target
    assert credentials["pass"] != old_account["password"]
    assert stored["metadata"]["labels"] == labels
    state = json.loads(accounts.read_text())
    assert state["users"]["insight_schedule"] == old_account
    assert state["users"][target]["password"] == credentials["pass"]
    assert sum("patched" in event for event in events) == 1
    assert not any(event.get("sql", "").startswith(("ALTER", "REVOKE")) for event in events)
    result, repeated, _ = hook(tmp_path, "reuse", overrides={"CONNECTOR_SCHEDULES_ACCOUNT_USER": target})
    assert result.returncode == 0, result.stderr
    assert json.loads(secret.read_text()) == stored
    assert not any("patched" in event or "created" in event for event in repeated)


def test_database_failure_retries_the_credentials_already_saved_in_the_secret(tmp_path: Path) -> None:
    result, _, secret = hook(tmp_path, "database-error")
    assert result.returncode == 1
    before = secret.read_text()
    result, events, _ = hook(tmp_path, "reuse")
    assert result.returncode == 0, result.stderr
    assert secret.read_text() == before
    assert not any("created" in event or "patched" in event for event in events)


@pytest.mark.parametrize("explicit_user", ["", "u" * 80])
def test_long_account_names_provision_and_reuse_bounded_roles(tmp_path: Path, explicit_user: str) -> None:
    overrides = {"CONNECTOR_SCHEDULES_ACCOUNT_USER": explicit_user, "CONNECTOR_SCHEDULES_DB_NAME": "d" * 64}
    result, _, _ = hook(tmp_path, "absent", overrides=overrides)
    assert result.returncode == 0, result.stderr
    state = json.loads((tmp_path / "accounts.json").read_text())
    assert len(next(iter(state["users"]))) == 80
    assert len(next(iter(state["roles"]))) <= 80
    result, _, _ = hook(tmp_path, "reuse", overrides=overrides)
    assert result.returncode == 0, result.stderr
    assert json.loads((tmp_path / "accounts.json").read_text()) == state


@pytest.mark.parametrize("failure", ["patch-error", "missing-version"])
def test_secret_upgrade_failure_preserves_the_old_credentials_and_does_no_database_work(
    tmp_path: Path, failure: str
) -> None:
    result, _, secret = hook(tmp_path, "absent")
    assert result.returncode == 0, result.stderr
    if failure == "missing-version":
        document = json.loads(secret.read_text())
        del document["metadata"]["resourceVersion"]
        secret.write_text(json.dumps(document))
    before = secret.read_text()
    result, events, _ = hook(tmp_path, failure, overrides={"CONNECTOR_SCHEDULES_ACCOUNT_USER": "example_new"})
    assert result.returncode == 1
    assert "schedule secret update failed" in result.stderr
    assert secret.read_text() == before
    assert not any("sql" in event for event in events)
    if failure == "missing-version":
        assert not any("patched" in event for event in events)


@pytest.mark.skipif(
    os.environ.get("SCHEDULE_GRANTS_LIVE") != "1",
    reason="set SCHEDULE_GRANTS_LIVE=1 for disposable MariaDB scope tests",
)
def test_live_two_installations_keep_credentials_and_refuse_foreign_grants() -> None:
    suffix = secrets.token_hex(6)
    databases = [f"schedule_one_{suffix}", f"schedule_two_{suffix}"]
    admin_env = {**os.environ, "CONNECTOR_SCHEDULES_DB_NAME": "mysql"}
    admin_env.pop("CONNECTOR_SCHEDULES_DB_NAME_FILE", None)
    environments = [
        {
            **admin_env,
            "CONNECTOR_SCHEDULES_DB_NAME": database,
            "CONNECTOR_SCHEDULES_ACCOUNT_USER": "",
            "CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD": secrets.token_hex(24),
        }
        for database in databases
    ]
    users = [account.account_name(env) for env in environments]
    roles = [account.bounded_name(user + "_role") for user in users]
    with connect(admin_env) as connection, connection.cursor() as cursor:
        try:
            for database in databases:
                cursor.execute(f"CREATE DATABASE `{database}`", None)
                cursor.execute(f"USE `{database}`", None)
                create_schedule_schema(cursor)
            for env in environments:
                account.provision(env)
            for env in reversed(environments):
                account.provision(env)
            assert users[0] != users[1] and roles[0] != roles[1]
            for env, user, other_database in zip(environments, users, reversed(databases), strict=True):
                runtime = {
                    **env,
                    "CONNECTOR_SCHEDULES_DB_USER": user,
                    "CONNECTOR_SCHEDULES_DB_PASSWORD": env["CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD"],
                }
                runtime.pop("CONNECTOR_SCHEDULES_DB_USER_FILE", None)
                runtime.pop("CONNECTOR_SCHEDULES_DB_PASSWORD_FILE", None)
                with connect(runtime) as reader, reader.cursor() as narrow:
                    narrow.execute("SELECT connector FROM connector_schedule_requests_v1", None)
                    assert list(narrow) == []
                    with pytest.raises(pymysql.MySQLError):
                        narrow.execute(f"SELECT connector FROM `{other_database}`.connector_schedule_requests_v1", None)
            cursor.execute(f"GRANT SELECT ON `{databases[1]}`.connector_schedule_requests_v1 TO %s@'%%'", (users[0],))
            cursor.execute(f"SHOW GRANTS FOR `{users[0]}`@`%`", None)
            before = list(cursor)
            with pytest.raises(AccountConflict):
                account.provision(environments[0])
            cursor.execute(f"SHOW GRANTS FOR `{users[0]}`@`%`", None)
            assert list(cursor) == before
        finally:
            for user in users:
                cursor.execute("DROP USER IF EXISTS %s@'%%'", (user,))
            for role in roles:
                cursor.execute(f"DROP ROLE IF EXISTS `{role}`", None)
            for database in databases:
                cursor.execute(f"DROP DATABASE IF EXISTS `{database}`", None)
