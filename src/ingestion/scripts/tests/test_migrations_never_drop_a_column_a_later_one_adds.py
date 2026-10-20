"""A column one migration drops must not be added back by a later one.

`apply-ch-migrations.sh` replays every file in `migrations/` on every deploy
with no ledger. A `DROP COLUMN IF EXISTS c` followed later by an
`ADD COLUMN IF NOT EXISTS c` on the same table therefore empties `c` on every
deploy: the drop finds the re-added column and removes it with its data, and
the add brings it back blank until the next rebuild refills it.
"""

from __future__ import annotations

import re

from test_migrations_never_need_a_bronze_table import MIGRATIONS, statements

ALTER_TABLE = re.compile(r"^ALTER TABLE ([\w.`]+) (.*)$")
DROP = re.compile(r"\bDROP COLUMN IF EXISTS ([\w`]+)")
ADD = re.compile(r"\bADD COLUMN IF NOT EXISTS ([\w`]+)")


def column_changes() -> list[tuple[str, str, str, str]]:
    """(file, verb, table, column) for every DROP/ADD COLUMN, in replay order."""
    changes = []
    for path in sorted(MIGRATIONS.glob("*.sql")):
        for statement in statements(path):
            alter = ALTER_TABLE.match(statement)
            if not alter:
                continue
            table = alter.group(1).replace("`", "")
            for verb, pattern in (("DROP", DROP), ("ADD", ADD)):
                changes.extend(
                    (path.name, verb, table, column.replace("`", "")) for column in pattern.findall(alter.group(2))
                )
    return changes


def test_no_column_is_dropped_and_then_added_back() -> None:
    dropped: dict[tuple[str, str], str] = {}
    offenders = []
    for name, verb, table, column in column_changes():
        if verb == "DROP":
            dropped[(table, column)] = name
        elif (table, column) in dropped:
            offenders.append(f"{table}.{column}: dropped in {dropped[(table, column)]}, added in {name}")
    assert not offenders, f"these columns are emptied on every deploy: {offenders}"


def test_the_parser_reads_both_verbs() -> None:
    """A parser that matched nothing would make the guard above pass by accident.

    Asserted over a fixture rather than over the corpus: the channel no longer
    carries a DROP COLUMN at all — #3562 deleted the silver class ALTERs — and
    a guard that only works while some migration happens to use a verb stops
    guarding the day the last one goes.
    """
    sql = "ALTER TABLE silver.class_x DROP COLUMN IF EXISTS gone, ADD COLUMN IF NOT EXISTS added Int64 AFTER id"
    alter = ALTER_TABLE.match(sql)
    assert alter is not None, "the ALTER TABLE form is no longer recognised"

    assert DROP.findall(alter.group(2)) == ["gone"]
    assert ADD.findall(alter.group(2)) == ["added"]


def test_the_corpus_is_read_at_all() -> None:
    """`column_changes` walking an empty set would be a silent pass."""
    assert list(MIGRATIONS.glob("*.sql")), f"no migration under {MIGRATIONS}"
