"""The run ledger's grant surface, asserted from the migration text.

The adversarial role test next door needs a live ClickHouse and is skipped
without one, so it never runs in CI. This reads the migration instead: no
server, no skip, and it fails the moment the ledger hands the query-path role
a write.

The role is not read-only in general — `presentation` and `product_usage` are
create/insert-only for it, which `test_presentation_role.py` pins. What must
hold here is narrower and absolute: on the ledger it reads and nothing more.
"""

from __future__ import annotations

import re
from pathlib import Path

MIGRATIONS = Path(__file__).resolve().parents[1] / "migrations"
LEDGER_MIGRATION = MIGRATIONS / "20260825100000_pipeline-run-ledger.sql"
READER_ROLE = "presentation_ro"

#: Anything that could change a recorded fact. The ledger is append-only and
#: the read surface is a reader; either half failing makes the page a liar.
WRITE_PRIVILEGES = ("INSERT", "ALTER", "DROP", "TRUNCATE", "DELETE", "UPDATE", "CREATE", "ALL")


def grant_statements() -> list[str]:
    text = LEDGER_MIGRATION.read_text()
    return [line.strip() for line in text.splitlines() if line.strip().upper().startswith("GRANT")]


def test_the_reader_is_granted_select_on_the_ledger() -> None:
    grants = grant_statements()

    assert any(
        re.search(rf"GRANT\s+SELECT\s+ON\s+ingestion_runs\.\*\s+TO\s+{READER_ROLE}", grant, re.I)
        for grant in grants
    ), f"the read surface cannot serve the page without it: {grants}"


def test_the_reader_is_granted_nothing_that_writes() -> None:
    for grant in grant_statements():
        if READER_ROLE not in grant:
            continue
        privileges = grant.upper().split(" ON ")[0]
        for privilege in WRITE_PRIVILEGES:
            assert not re.search(rf"\b{privilege}\b", privileges), (
                f"{privilege} would let the read surface change what it reports: {grant}"
            )


def test_no_other_role_is_handed_the_ledger_by_this_migration() -> None:
    # The writers authenticate as the owning ingestion user; a grant here would
    # be a second write path nobody is watching.
    for grant in grant_statements():
        assert READER_ROLE in grant, f"unexpected grant in the ledger migration: {grant}"
