"""DDL on a relation in a `bronze_*` database carries the cluster clause.

`bronze_*` is created by the Airbyte destination as a plain Atomic database, so
nothing replays a CREATE, DROP, RENAME or EXCHANGE issued there: without `ON
CLUSTER` the statement reaches the one replica the deploy connected to. Our
own databases are Replicated and the server rejects the clause inside them, so
it must appear on the bronze statements and nowhere else. The numbered
migrations spell it as the `/*@@ON_CLUSTER@@*/` marker; the heals in
apply-ch-migrations.sh call `ch_on_cluster`.

ALTER, TRUNCATE and INSERT on an existing Replicated table replicate by
themselves and take no clause.
"""

from __future__ import annotations

import re
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
MIGRATIONS = SCRIPTS / "migrations"
HEALS = SCRIPTS / "apply-ch-migrations.sh"

ON_CLUSTER_MARKER = "/*@@ON_CLUSTER@@*/"
ON_CLUSTER_CALL = "$(ch_on_cluster)"

RELATION_DDL = re.compile(r"^\s*(DROP|CREATE|RENAME|EXCHANGE)\s+(TABLE|TABLES|VIEW|DICTIONARY)\b", re.IGNORECASE)
NAMES_BRONZE = re.compile(r"\bbronze_\w*\.|\$\{copy\}", re.IGNORECASE)


def migration_statements() -> list[tuple[str, str]]:
    found = []
    for path in sorted(MIGRATIONS.glob("*.sql")):
        body = " ".join(line for line in path.read_text().splitlines() if not line.strip().startswith("--"))
        found.extend((path.name, " ".join(part.split())) for part in body.split(";") if part.strip())
    assert found, f"no statements under {MIGRATIONS}"

    return found


def heal_ddl_lines() -> list[str]:
    lines = [line for line in HEALS.read_text().splitlines() if RELATION_DDL.match(line)]
    assert lines, f"{HEALS.name} issues no relation DDL"

    return lines


def test_every_bronze_relation_ddl_in_a_migration_carries_the_marker() -> None:
    offenders = [
        (name, statement)
        for name, statement in migration_statements()
        if RELATION_DDL.match(statement) and NAMES_BRONZE.search(statement) and ON_CLUSTER_MARKER not in statement
    ]
    assert not offenders, f"bronze DDL without {ON_CLUSTER_MARKER}: {offenders}"


def test_the_marker_qualifies_nothing_but_a_bronze_relation() -> None:
    offenders = [
        (name, statement)
        for name, statement in migration_statements()
        if ON_CLUSTER_MARKER in statement and not NAMES_BRONZE.search(statement)
    ]
    assert not offenders, f"{ON_CLUSTER_MARKER} on a relation whose Replicated database rejects it: {offenders}"


def test_every_bronze_relation_ddl_in_a_heal_names_the_cluster() -> None:
    offenders = [line for line in heal_ddl_lines() if NAMES_BRONZE.search(line) and ON_CLUSTER_CALL not in line]
    assert not offenders, f"bronze DDL without {ON_CLUSTER_CALL}: {offenders}"


def test_no_heal_names_the_cluster_on_a_relation_outside_bronze() -> None:
    offenders = [line for line in heal_ddl_lines() if ON_CLUSTER_CALL in line and not NAMES_BRONZE.search(line)]
    assert not offenders, f"{ON_CLUSTER_CALL} on a relation whose Replicated database rejects it: {offenders}"
