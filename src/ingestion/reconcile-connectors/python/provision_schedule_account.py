from __future__ import annotations

import hashlib
import os
import re
import sys
from collections.abc import Mapping
from typing import TYPE_CHECKING

from schedule_database import connect, setting
from schedule_failure import AccountConflict, ScheduleFailure

if TYPE_CHECKING:
    from pymysql.cursors import SSCursor


def bounded_name(name: str) -> str:
    if len(name) <= 80:
        return name
    suffix = hashlib.sha256(name.encode()).hexdigest()[:12]
    return f"{name[:67]}_{suffix}"


def account_name(env: Mapping[str, str]) -> str:
    return env.get("CONNECTOR_SCHEDULES_ACCOUNT_USER") or bounded_name(
        "insight_schedule_" + setting(env, "CONNECTOR_SCHEDULES_DB_NAME")
    )


def allowed_grants(user: str, role: str, database: str, *, for_role: bool) -> set[str]:
    principal = f"`{role}`" if for_role else f"`{user}`@`%`"
    if for_role:
        return {
            f"GRANT SELECT ON `{database}`.`connector_schedule_requests_v1` TO {principal}",
            f"GRANT SELECT (`connector`, `source_id`), DELETE ON `{database}`.`connector_schedules` TO {principal}",
            f"GRANT SELECT (`source_id`, `connector`), DELETE ON `{database}`.`connector_schedules` TO {principal}",
        }
    return {f"GRANT `{role}` TO {principal}", f"SET DEFAULT ROLE `{role}` FOR {principal}"}


def check_grants(grants: list[str], user: str, role: str, database: str, *, for_role: bool) -> None:
    principal = f"`{role}`" if for_role else f"`{user}`@`%`"
    usage = re.escape(f"GRANT USAGE ON *.* TO {principal}") + r"(?: IDENTIFIED BY PASSWORD '[*A-F0-9]+')?"
    allowed = allowed_grants(user, role, database, for_role=for_role)
    if any(grant not in allowed and not re.fullmatch(usage, grant) for grant in grants):
        raise AccountConflict


def existing_grants(cursor: SSCursor, name: str, *, for_role: bool) -> list[str] | None:
    cursor.execute("SELECT is_role FROM mysql.user WHERE User=%s AND Host=%s", (name, "" if for_role else "%"))
    rows = list(cursor)
    if not rows:
        return None
    if rows != [("Y" if for_role else "N",)]:
        raise AccountConflict
    principal = f"`{name}`" if for_role else f"`{name}`@`%`"
    cursor.execute(f"SHOW GRANTS FOR {principal}", None)
    return [grant for (grant,) in cursor]


def check_credentials(env: Mapping[str, str], user: str, password: str) -> None:
    import pymysql

    runtime = dict(env)
    for name in ["USER", "PASSWORD", "NAME"]:
        runtime.pop(f"CONNECTOR_SCHEDULES_DB_{name}_FILE", None)
    runtime.update(
        CONNECTOR_SCHEDULES_DB_USER=user,
        CONNECTOR_SCHEDULES_DB_PASSWORD=password,
        CONNECTOR_SCHEDULES_DB_NAME="information_schema",
    )
    try:
        with connect(runtime) as connection, connection.cursor() as cursor:
            cursor.execute("SELECT CURRENT_USER()", None)
            if list(cursor) != [(f"{user}@%",)]:
                raise AccountConflict
    except pymysql.MySQLError as exc:
        if exc.args[0] == 1045:
            raise AccountConflict from None
        raise


def provision(env: Mapping[str, str]) -> None:
    user = account_name(env)
    password = setting(env, "CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD")
    database = setting(env, "CONNECTOR_SCHEDULES_DB_NAME")
    if not re.fullmatch(r"[a-zA-Z0-9_]{1,64}", database) or not re.fullmatch(r"[a-zA-Z0-9_]{1,80}", user):
        raise ValueError("invalid schedule account configuration")
    role = bounded_name(user + "_role")
    with connect(env) as connection, connection.cursor() as cursor:
        user_grants = existing_grants(cursor, user, for_role=False)
        role_grants = existing_grants(cursor, role, for_role=True)
        for grants, for_role in [(user_grants, False), (role_grants, True)]:
            if grants is not None:
                check_grants(grants, user, role, database, for_role=for_role)

        cursor.execute(
            "SELECT User, Host FROM mysql.roles_mapping WHERE Role=%s "
            "AND NOT (CONCAT(User, '@', Host)=CURRENT_USER() AND Admin_option='Y')",
            (role,),
        )
        if any(member != (user, "%") for member in cursor):
            raise AccountConflict
        if user_grants is not None:
            check_credentials(env, user, password)

        if user_grants is None:
            cursor.execute("CREATE USER %s@'%%' IDENTIFIED BY %s", (user, password))
        if role_grants is None:
            cursor.execute(f"CREATE ROLE `{role}`", None)
        cursor.execute(f"GRANT SELECT ON `{database}`.connector_schedule_requests_v1 TO `{role}`", None)
        cursor.execute(
            f"GRANT SELECT (connector, source_id), DELETE ON `{database}`.connector_schedules TO `{role}`", None
        )
        cursor.execute(f"GRANT `{role}` TO %s@'%%'", (user,))
        cursor.execute(f"SET DEFAULT ROLE `{role}` FOR %s@'%%'", (user,))


def main() -> int:
    try:
        provision(os.environ)
        return 0
    except Exception as exc:
        print(str(ScheduleFailure("account provisioning", exc)), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
