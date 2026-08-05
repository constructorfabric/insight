"""
MariaDB analytics seed: the catalogue rows no endpoint can create.

One table the product provisions by migration rather than through its API, so a
suite has nothing to create it with:

  metric_definitions  a TENANT-scoped row overriding a product default. The
                      listing is supposed to resolve the tenant's label over the
                      product's; with no tenant row anywhere, nothing proves it.

Seeded here rather than inserted by a test fixture on purpose. The compose-stand
suite holds no database connection — that would hand every test a back door
around the deployed path it exists to exercise — so anything a test needs and no
endpoint creates has to be seeded, and then NAMED IN THE MANIFEST. A test reads
the name from there; it never hardcodes one.

Runs after analytics has migrated: the table is created by its SeaORM
migrations at startup, not by this seed.
"""

from __future__ import annotations

import logging
import os
import uuid as uuid_mod
from collections.abc import Iterator
from contextlib import contextmanager
from typing import Any

import pymysql

LOG = logging.getLogger("seed.analytics")

#: Deterministic id, so re-seeding an un-torn-down stand replaces its own row
#: instead of accumulating a new one every run.
DEFINITION_ROW_ID = "e1e1e1e1-0000-4000-8000-000000000010"

#: The label constant lives in `manifest` rather than here: `PROFILE.md` is
#: rendered by a tool that must import no third-party package, and this module
#: needs pymysql. The manifest owns the NAMES; this module owns writing the rows.
from manifest import OVERRIDE_LABEL  # noqa: E402


def _bin(u: str) -> bytes:
    """UUID string → 16 raw bytes, matching the identity seed's convention."""
    return uuid_mod.UUID(u).bytes


@contextmanager
def _connect() -> Iterator[pymysql.connections.Connection]:
    conn = pymysql.connect(
        host=os.environ.get("MARIADB_HOST", "mariadb"),
        port=int(os.environ.get("MARIADB_PORT", "3306")),
        user=os.environ.get("MARIADB_USER", "insight"),
        password=os.environ.get("MARIADB_PASSWORD", "insight-local"),
        # Not MARIADB_DB: that one names the IDENTITY database. These tables
        # belong to analytics, which owns a database of its own.
        database=os.environ.get("MARIADB_ANALYTICS_DB", "analytics"),
        autocommit=False,
        cursorclass=pymysql.cursors.Cursor,
    )
    try:
        yield conn
        conn.commit()
    except Exception:
        conn.rollback()
        raise
    finally:
        conn.close()


#: MySQL's "The value specified for generated column … has been ignored" — the
#: only warning the write below is allowed to raise. See `_require_clean_ignore`.
_GENERATED_COLUMN_IGNORED = 1906


def _require_clean_ignore(cur: pymysql.cursors.Cursor, rows: int) -> None:
    """Check that `INSERT IGNORE` ignored only what it was meant to.

    The IGNORE exists for exactly one reason: `SELECT *` carries the generated
    `tenant_id_sentinel` across, no statement may assign a generated column, and
    IGNORE is what demotes that refusal to a warning so the engine recomputes
    the value instead. It is a blunt instrument though — it would just as
    happily turn a duplicate key or a truncated value into a warning and skip
    the row — so what it swallowed is inspected rather than trusted.

    `rows` is read by the caller before this runs: `SHOW WARNINGS` resets
    `cur.rowcount`.
    """
    if rows != 1:
        raise RuntimeError(
            f"the definition override wrote {rows} row(s), expected 1 — the INSERT was "
            "skipped, which IGNORE would otherwise have hidden."
        )
    cur.execute("SHOW WARNINGS")
    unexpected = [w for w in cur.fetchall() if int(w[1]) != _GENERATED_COLUMN_IGNORED]
    if unexpected:
        raise RuntimeError(
            f"the definition override raised warnings beyond the expected generated-column "
            f"one: {unexpected}"
        )


def _child_row_id(table: str, source_id: bytes) -> bytes:
    """A deterministic id for a cloned child row.

    Derived from the override's own id so re-seeding an un-torn-down stand
    replaces the same rows rather than accumulating a fresh set every run —
    the same reason the parent id is a constant.
    """
    return uuid_mod.uuid5(uuid_mod.UUID(DEFINITION_ROW_ID), f"{table}:{source_id.hex()}").bytes


#: The child clone is done BY THE DATABASE — `CREATE … LIKE` plus
#: `INSERT … SELECT *` copy whatever the schema currently holds, so this module
#: never enumerates the child tables' columns and cannot drift from them.
#:
#: Plain `LIKE` works here because neither child table has a generated column.
#: The parent does, which is why its copy is staged with `AS SELECT` and written
#: with `INSERT IGNORE` instead — see `_OVERRIDE_STAGE`.
#:
#: One temp-table NAME serves both tables, so every statement that touches only
#: the copy is shared below and just the three naming a real table differ.
_CHILD_CLONE_SQL: dict[str, tuple[str, str, str]] = {
    "metric_definition_inputs": (
        "CREATE TEMPORARY TABLE seed_child_clone LIKE metric_definition_inputs",
        "INSERT INTO seed_child_clone SELECT * FROM metric_definition_inputs WHERE metric_definition_id = %s",
        "REPLACE INTO metric_definition_inputs SELECT * FROM seed_child_clone",
    ),
    "metric_definition_dimensions": (
        "CREATE TEMPORARY TABLE seed_child_clone LIKE metric_definition_dimensions",
        "INSERT INTO seed_child_clone SELECT * FROM metric_definition_dimensions WHERE metric_definition_id = %s",
        "REPLACE INTO metric_definition_dimensions SELECT * FROM seed_child_clone",
    ),
}

#: Statements that touch only the copy, identical for either table.
#: `CREATE TEMPORARY TABLE` and `DROP TEMPORARY TABLE` are the documented
#: exceptions to MySQL's implicit-commit rule, so none of this escapes the
#: transaction `_connect` opens.
_CLONE_DROP = "DROP TEMPORARY TABLE IF EXISTS seed_child_clone"
_CLONE_REKEY_PARENT = "UPDATE seed_child_clone SET metric_definition_id = %s"
_CLONE_SELECT_IDS = "SELECT id FROM seed_child_clone"
_CLONE_REKEY_ID = "UPDATE seed_child_clone SET id = %s WHERE id = %s"


def _clone_children(cur: pymysql.cursors.Cursor, table: str, src: bytes, dst: bytes) -> int:
    """Re-key one child table's rows onto the cloned definition.

    Through a temporary copy rather than a read-then-write loop, so the column
    inventory stays the database's business. The ids are still chosen here —
    `_child_row_id` keeps them deterministic — and applied as bound UPDATEs.
    """
    create, fill, write_back = _CHILD_CLONE_SQL[table]
    cur.execute(_CLONE_DROP)
    cur.execute(create)
    cur.execute(fill, (src,))
    cur.execute(_CLONE_REKEY_PARENT, (dst,))

    # Re-keyed one row at a time: the id is derived from the SOURCE id, which
    # only Python knows how to compute. Safe to do in place because a derived id
    # never collides with a source id still waiting its turn.
    cur.execute(_CLONE_SELECT_IDS)
    child_ids = [row[0] for row in cur.fetchall()]
    for child_id in child_ids:
        cur.execute(_CLONE_REKEY_ID, (_child_row_id(table, child_id), child_id))

    cur.execute(write_back)
    cur.execute(_CLONE_DROP)
    return len(child_ids)


#: The override is staged in a temporary copy of the chosen product row, mutated
#: there, and written back — so this module never enumerates
#: `metric_definitions`' columns either, and no statement here is composed at
#: runtime.
#:
#: `AS SELECT` rather than `LIKE`: it materialises the generated
#: `tenant_id_sentinel` as an ordinary column, which is what lets the row go back
#: with a bare `SELECT *`. `LIKE` would carry the column's GENERATED attribute
#: over and the staging INSERT would be the statement that failed instead.
#:
#: `INSERT IGNORE` because even materialised, `SELECT *` still offers a value for
#: the target's generated column, and only IGNORE demotes the refusal to a
#: warning and lets the engine recompute it from the new `tenant_id`.
#: `_require_clean_ignore` then checks that nothing ELSE was ignored.
_OVERRIDE_DROP = "DROP TEMPORARY TABLE IF EXISTS seed_definition_override"
_OVERRIDE_STAGE = (
    "CREATE TEMPORARY TABLE seed_definition_override AS "
    "SELECT * FROM metric_definitions WHERE tenant_id IS NULL ORDER BY metric_key LIMIT 1"
)
_OVERRIDE_SOURCE = "SELECT id, metric_key, updated_at FROM seed_definition_override"
#: `updated_at` is re-set to the source's own value: `AS SELECT` carries the
#: column's ON UPDATE clause across too, so without this the UPDATE below would
#: stamp the copy with the current time and the clone would stop being faithful
#: in the one column nobody would think to check.
_OVERRIDE_APPLY = (
    "UPDATE seed_definition_override "
    "SET id = %s, tenant_id = %s, label = %s, origin = 'custom', updated_at = %s"
)
#: REPLACE's delete half, on its own. The FK from either child table is
#: ON DELETE CASCADE, so this clears the previous run's children as well —
#: which is why the clone below runs after it, exactly as it did under REPLACE.
_OVERRIDE_CLEAR = "DELETE FROM metric_definitions WHERE id = %s"
_OVERRIDE_WRITE = "INSERT IGNORE INTO metric_definitions SELECT * FROM seed_definition_override"


def seed_definition_override(
    cur: pymysql.cursors.Cursor, tenant_uuid: str
) -> dict[str, str] | None:
    """Override one product definition's label for this tenant.

    WHICH key is chosen at seed time rather than pinned here: the product's
    definitions come from migrations and this seed must not carry a second copy
    of that list to go stale. The lowest key by sort order is deterministic
    given the same migrations, and the choice is recorded in the manifest so the
    test reads it instead of guessing.

    A FAITHFUL clone — every column, plus the input and dimension rows that hang
    off the definition. A tenant row SHADOWS the product default rather than
    decorating it, so a partial copy does not produce a definition that is
    mostly right: it produces one the resolver rejects outright (`missing Value
    input for …`), and every metric-results call touching that key answers 500.
    The override is meant to change the LABEL and nothing else, so everything
    else has to come across.

    Returns None when the product has no definitions at all — a stand whose
    migrations have not run, which is the migrations' problem to report, not
    this seed's to fail on.

    Raises if some OTHER row already holds this tenant's (tenant, metric_key):
    the write clears its own id and nothing else, where REPLACE used to delete
    whatever stood in the way. Nothing on a seeded stand puts a row there, so
    finding one means an assumption broke and is worth stopping for.
    """
    cur.execute(_OVERRIDE_DROP)
    cur.execute(_OVERRIDE_STAGE)
    cur.execute(_OVERRIDE_SOURCE)
    staged = cur.fetchone()
    if staged is None:
        cur.execute(_OVERRIDE_DROP)
        LOG.warning("  metric_definitions: no product definitions to override — skipped")
        return None

    source_id, metric_key, source_updated_at = staged[0], str(staged[1]), staged[2]
    override_id = _bin(DEFINITION_ROW_ID)

    cur.execute(
        _OVERRIDE_APPLY, (override_id, _bin(tenant_uuid), OVERRIDE_LABEL, source_updated_at)
    )
    cur.execute(_OVERRIDE_CLEAR, (override_id,))
    cur.execute(_OVERRIDE_WRITE)
    _require_clean_ignore(cur, cur.rowcount)
    cur.execute(_OVERRIDE_DROP)

    cloned = {
        table: _clone_children(cur, table, source_id, override_id)
        for table in ("metric_definition_inputs", "metric_definition_dimensions")
    }

    LOG.info(
        "  metric_definitions\n    %s → %r (%s)",
        metric_key,
        OVERRIDE_LABEL,
        ", ".join(f"{n} {t.removeprefix('metric_definition_')}" for t, n in cloned.items()),
    )
    return {"metric_key": metric_key, "label": OVERRIDE_LABEL}


def run() -> dict[str, Any]:
    """Seed both catalogues; return what was written, for the manifest.

    Returned rather than written to the manifest here because `build_manifest`
    is pure — it reads its own sources and nothing else, so a fact discovered at
    seed time has to be handed to it.
    """
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s %(message)s",
    )
    tenant = os.environ.get("TENANT_DEFAULT_ID", "00000000-df51-5b42-9538-d2b56b7ee953")

    LOG.info("analytics catalogue seed (tenant %s)", tenant)
    with _connect() as conn:
        cur = conn.cursor()
        override = seed_definition_override(cur, tenant)

    return {"definition_override": override}
