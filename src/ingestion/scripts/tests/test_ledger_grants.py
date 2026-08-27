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

Read as STATEMENTS, not lines. A guard that greps for lines both starting with
`GRANT` and naming the database is blind to three ways in, each of which leaves
such a guard green: a wildcard (`GRANT ALL ON *.*`), a grant split across two
lines, and a grant of some other role that itself carries writes.
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

#: Objects a grant can name that reach the ledger. Upper-cased, because the
#: statements are: a case mismatch here would make every check below match
#: nothing and pass without asserting anything.
LEDGER_OBJECTS = (
    f"{LEDGER_DB.upper()}.*",
    f"{LEDGER_DB.upper()}.SYNC_EVENTS",
    "*.*",
)


def statements() -> list[str]:
    """The file as statements: comments dropped, whitespace collapsed, upper-cased."""
    lines = [
        line for line in ROLE_SQL.read_text().splitlines() if not line.strip().startswith("--")
    ]
    body = " ".join(lines)
    return [" ".join(part.split()).upper() for part in body.split(";") if part.strip()]


def grants() -> list[str]:
    return [stmt for stmt in statements() if stmt.startswith("GRANT ")]


def _privileges_and_object(grant: str) -> tuple[str, str] | None:
    """`GRANT SELECT ON db.* TO role` -> `("SELECT", "DB.*")`; None for a role grant."""
    if " ON " not in grant:
        return None
    head, _, tail = grant.partition(" ON ")
    obj = tail.split(" TO ")[0].strip()
    return head[len("GRANT ") :].strip(), obj


def test_the_reader_is_granted_select_on_the_ledger() -> None:
    reaching = [
        grant
        for grant in grants()
        if (parsed := _privileges_and_object(grant))
        and parsed[1] == f"{LEDGER_DB.upper()}.*"
        and READER_ROLE.upper() in grant
    ]

    assert reaching, f"{ROLE_SQL.name} grants the reader nothing on {LEDGER_DB}"
    assert any("SELECT" in grant for grant in reaching), reaching


def test_nothing_that_writes_the_ledger_is_granted_to_the_reader() -> None:
    """Covers the wildcard and the multi-line spellings as well as the plain one."""
    for grant in grants():
        parsed = _privileges_and_object(grant)
        if parsed is None:
            continue
        privileges, obj = parsed
        if obj not in LEDGER_OBJECTS or READER_ROLE.upper() not in grant:
            continue
        for write in WRITE_PRIVILEGES:
            assert write not in privileges, (
                f"the query path must not be able to change what it reports on: {grant}"
            )


def test_the_reader_carries_no_other_role() -> None:
    """A role grant names no database, so a guard reading objects cannot see it —
    and whatever that role carries, the reader carries too."""
    for grant in grants():
        if _privileges_and_object(grant) is not None:
            continue
        assert READER_ROLE.upper() not in grant, (
            f"the reader must hold only its own grants: {grant}"
        )


def test_nothing_revokes_the_reader_s_own_access() -> None:
    """A REVOKE after the grant breaks the page without failing any grant check."""
    for stmt in statements():
        assert not stmt.startswith("REVOKE "), f"unexpected REVOKE: {stmt}"


def test_the_ledger_is_granted_in_exactly_one_statement() -> None:
    """A second statement is how a write slips in beside a legitimate read."""
    reaching = [
        grant
        for grant in grants()
        if (parsed := _privileges_and_object(grant)) and parsed[1] in LEDGER_OBJECTS
    ]
    assert len(reaching) == 1, reaching
