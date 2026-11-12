from __future__ import annotations

from unittest.mock import MagicMock

import provision_schedule_account as account
import pytest


@pytest.mark.parametrize("for_role", [False, True])
def test_grant_reads_preserve_principals_with_pymysql_parameter_formatting(database: MagicMock, for_role: bool) -> None:
    principal = "`example_role`" if for_role else "`example`@`%`"
    grant = f"GRANT USAGE ON *.* TO {principal}"
    database.__iter__.side_effect = [iter([("Y" if for_role else "N",)]), iter([(grant,)])]
    queries = []

    def execute(query: str, args: tuple[str, ...] | None = None) -> int:
        formatted = query % args if args is not None else query
        queries.append(formatted)
        return 1

    database.execute.side_effect = execute

    result = account.existing_grants(database, "example_role" if for_role else "example", for_role=for_role)

    assert result == [grant]
    assert queries[-1] == f"SHOW GRANTS FOR {principal}"
