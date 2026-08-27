"""The sync ledger's grant surface, asserted from the role definition's text.

The adversarial role test next door needs a live ClickHouse and skips without
one, so it never runs in CI. This reads the SQL instead: no server, no skip, and
it fails the moment the ledger hands the query-path role a write.

The role is not read-only in general — `presentation` is create/insert-only for
it and `product_usage` is insert-only, both of which `test_presentation_role.py`
pins. What must hold here is narrower and absolute: on the ledger the query path
reads and nothing more. Its writer is the reconcile loop, under different
credentials entirely, so a write grant here would be a grant nothing uses — and
the surface reporting on ingestion could edit its own evidence.
"""

from __future__ import annotations

from pathlib import Path

BOOTSTRAP = Path(__file__).resolve().parents[1] / "bootstrap-db"
ROLE_SQL = BOOTSTRAP / "presentation-role.sql"
LEDGER_DB = "ingestion_history"
READER_ROLE = "presentation_ro"

#: Anything that could change a recorded fact. The ledger is append-only and the
#: read surface is a reader; either half failing makes the page a liar.
WRITE_PRIVILEGES = (
    "INSERT",
    "ALTER",
    "DROP",
    "TRUNCATE",
    "DELETE",
    "UPDATE",
    "CREATE",
    "ALL",
)


def ledger_grants() -> list[str]:
    """Every GRANT statement that names the ledger database."""
    text = ROLE_SQL.read_text()
    return [
        " ".join(line.split())
        for line in text.splitlines()
        if line.strip().upper().startswith("GRANT") and LEDGER_DB in line
    ]


def test_the_reader_is_granted_select_on_the_ledger() -> None:
    grants = ledger_grants()

    assert grants, f"{ROLE_SQL.name} grants the reader nothing on {LEDGER_DB}"
    assert any(
        "SELECT" in grant.upper() and READER_ROLE in grant for grant in grants
    ), f"the read surface cannot read the ledger: {grants}"


def test_the_reader_is_granted_nothing_that_writes() -> None:
    for grant in ledger_grants():
        privileges = grant.upper().split(" ON ", 1)[0]
        for write in WRITE_PRIVILEGES:
            assert write not in privileges, (
                f"the query path must not be able to change what it reports on: "
                f"{grant}"
            )


def test_the_ledger_database_is_named_exactly_once() -> None:
    """A second grant is how a write slips in beside a legitimate read."""
    assert len(ledger_grants()) == 1, ledger_grants()
