"""A numbered migration must survive a deploy on which no bronze table exists.

`apply-ch-migrations.sh` replays every file in `migrations/` on every deploy,
before any connector has necessarily synced, and ClickHouse has no table-level
`IF EXISTS` on `ALTER` — so a bare `ALTER TABLE bronze_x.y` aborts the Helm hook
wherever that connector never ran. A bronze change belongs in a
`ch_table_exists`-guarded heal instead (AGENTS.md, "Warehouse contract
changes").

Read as STATEMENTS, not lines: the statements here span several lines each, and
a line-wise guard is blind to the common shape — the relation on the line after
the verb.
"""

from __future__ import annotations

import re
from pathlib import Path

MIGRATIONS = Path(__file__).resolve().parents[1] / "migrations"

#: Only form that is safe unguarded: it asks for the table and accepts its absence.
GUARDED = re.compile(r"^DROP TABLE IF EXISTS\b")

NAMES_BRONZE = re.compile(r"\bBRONZE[_.]")


def statements(path: Path) -> list[str]:
    """The file as statements: comment lines dropped, whitespace collapsed, upper-cased."""
    body = " ".join(line for line in path.read_text().splitlines() if not line.strip().startswith("--"))
    return [" ".join(part.split()).upper() for part in body.split(";") if part.strip()]


def test_no_numbered_migration_names_a_bronze_relation_unguarded() -> None:
    offenders = [
        (path.name, statement)
        for path in sorted(MIGRATIONS.glob("*.sql"))
        for statement in statements(path)
        if NAMES_BRONZE.search(statement) and not GUARDED.match(statement)
    ]
    assert not offenders, (
        "a numbered migration needs a bronze table to exist; move it behind a "
        f"ch_table_exists guard in apply-ch-migrations.sh: {offenders}"
    )


def test_the_inventory_itself_is_not_empty() -> None:
    """A glob that matches nothing would make the guard above pass by accident."""
    assert list(MIGRATIONS.glob("*.sql")), f"no migrations found under {MIGRATIONS}"
