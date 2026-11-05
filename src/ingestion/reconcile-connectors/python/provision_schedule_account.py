from __future__ import annotations

import os
import re
import sys
from collections.abc import Mapping

from schedule_database import connect, setting
from schedule_failure import ScheduleFailure


def provision(env: Mapping[str, str]) -> None:
    user = setting(env, "CONNECTOR_SCHEDULES_ACCOUNT_USER")
    password = setting(env, "CONNECTOR_SCHEDULES_ACCOUNT_PASSWORD")
    database = setting(env, "CONNECTOR_SCHEDULES_DB_NAME")
    if not re.fullmatch(r"[a-zA-Z0-9_]{1,64}", database) or not re.fullmatch(r"[a-zA-Z0-9_]{1,64}", user):
        raise ValueError("invalid schedule account configuration")
    role = user + "_role"
    with connect(env) as connection, connection.cursor() as cursor:
        cursor.execute("CREATE USER IF NOT EXISTS %s@'%%' IDENTIFIED BY %s", (user, password))
        cursor.execute("ALTER USER %s@'%%' IDENTIFIED BY %s", (user, password))
        cursor.execute(f"CREATE ROLE IF NOT EXISTS `{role}`", ())
        cursor.execute(f"REVOKE ALL PRIVILEGES, GRANT OPTION FROM `{role}`", ())
        cursor.execute("REVOKE ALL PRIVILEGES, GRANT OPTION FROM %s@'%%'", (user,))
        cursor.execute("SELECT Role FROM mysql.roles_mapping WHERE User=%s AND Host=%s", (user, "%"))
        roles = list(cursor)
        for (assigned,) in roles:
            quoted = assigned.replace("`", "``")
            cursor.execute(f"REVOKE `{quoted}` FROM %s@'%%'", (user,))
        cursor.execute(f"GRANT SELECT ON `{database}`.connector_schedule_requests_v1 TO `{role}`", ())
        cursor.execute(
            f"GRANT SELECT, INSERT, UPDATE ON `{database}`.connector_schedule_acknowledgements TO `{role}`", ()
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
