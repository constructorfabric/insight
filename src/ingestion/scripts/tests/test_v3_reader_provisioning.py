"""The assistant's ClickHouse account, asserted from the provisioning script.

`insight_v3_reader` is what the custom surfaces query as, and its whole point is
that it cannot write: every privilege arrives through `insight_v3_ro`, whose
read-only contract `test_ledger_grants.py` pins from the role SQL.

A privilege granted straight to the user would sit outside that contract and
outside that test — the role would still read as read-only while the account
querying with it could write. So this reads the script's statements and refuses
any grant to the user that is not the role itself.
"""

from __future__ import annotations

import re
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
PROVISION_SCRIPT = SCRIPTS / "bootstrap-db" / "provision-v3-access.sh"
MIGRATIONS = SCRIPTS / "apply-ch-migrations.sh"

READER_ROLE = "insight_v3_ro"

#: Anything that could change data. `ALL` covers `GRANT ALL PRIVILEGES`.
WRITE_PRIVILEGES = (
    "INSERT",
    "ALTER",
    "DROP",
    "TRUNCATE",
    "DELETE",
    "UPDATE",
    "CREATE TABLE",
    "ALL",
)


def statements() -> list[str]:
    """The script's SQL, one statement per entry, upper-cased."""
    body = PROVISION_SCRIPT.read_text()
    heredoc = re.search(r"run_ch <<SQL\n(.*?)\nSQL\n", body, re.S)
    assert heredoc, "the script no longer provisions through a run_ch heredoc"

    return [
        " ".join(statement.split()).upper()
        for statement in heredoc.group(1).split(";")
        if statement.strip()
    ]


def test_the_reader_is_granted_the_read_only_role_and_defaults_to_it() -> None:
    granted = [s for s in statements() if s.startswith("GRANT ")]

    assert granted, "the reader is granted nothing, so it can read nothing"
    assert all(
        s.startswith(f"GRANT {READER_ROLE.upper()} TO ") for s in granted
    ), f"every grant must be the {READER_ROLE} role itself: {granted}"
    assert any(
        s.startswith("ALTER USER") and f"DEFAULT ROLE {READER_ROLE.upper()}" in s
        for s in statements()
    ), f"the reader must default to {READER_ROLE}, or it queries with no role at all"


def test_no_privilege_reaches_the_reader_directly() -> None:
    for statement in statements():
        if not statement.startswith("GRANT "):
            continue
        for privilege in WRITE_PRIVILEGES:
            assert privilege not in statement, (
                f"{privilege} is granted to the account itself, outside the "
                f"read-only role: {statement}"
            )


def test_the_migration_run_provisions_the_reader() -> None:
    called = MIGRATIONS.read_text()

    assert PROVISION_SCRIPT.name in called, (
        "nothing calls the provisioning script, so a deployment leaves the "
        "assistant querying as the service's own read-write user"
    )
