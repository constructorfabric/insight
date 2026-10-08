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


def test_the_parser_sees_both_verbs() -> None:
    """A parser that matched nothing would make the guard above pass by accident."""
    verbs = {verb for _, verb, _, _ in column_changes()}
    assert verbs == {"DROP", "ADD"}, f"expected both verbs, saw {verbs}"
