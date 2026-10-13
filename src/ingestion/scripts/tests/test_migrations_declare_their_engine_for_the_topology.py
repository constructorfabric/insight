"""A numbered migration states its engine for the declared topology, never flat.

The dbt models route every engine through the `insight_engine` macro, so on a
clustered warehouse they are created `Replicated*` and their rows reach every
replica. The numbered migrations are plain SQL with no template engine above
them, so the same decision reaches them as a marker the deploy expands:
`ENGINE = /*@@REPLICATED@@*/ReplacingMergeTree(_version)`.

A migration whose engine stays the plain family compiles, deploys and looks
healthy. What it leaves behind is a table that exists on every replica — the
database engine replays the DDL — holding whatever rows reached the node that
answered. Two of these relations are also created by a dbt model, which would
be rendering the replicated family while the migration rendered the plain one,
so whichever ran first would decide and `IF NOT EXISTS` would hide it.

The marker is a SQL comment so the file stays executable unrendered, which is
how the data-path harness and the services' live tests apply it — on a single
node, where the plain family is the right answer.

Engines outside the MergeTree family are not covered: they are not replicated
by prefixing a name, and a migration that needs one needs its own answer.
"""

from __future__ import annotations

import re
from pathlib import Path

MIGRATIONS = Path(__file__).resolve().parents[1] / "migrations"

TOPOLOGY_MARKER = "/*@@REPLICATED@@*/"

#: `ENGINE = <something>MergeTree`, with whatever precedes the family captured
#: so an unmarked spelling is what fails.
ENGINE_CLAUSE = re.compile(r"ENGINE\s*=\s*(\S*?)(\w*MergeTree)\b", re.IGNORECASE)


def migrations() -> list[Path]:
    found = sorted(MIGRATIONS.glob("*.sql"))
    assert found, f"no migrations under {MIGRATIONS}"

    return found


def test_every_mergetree_engine_is_rendered_from_the_topology() -> None:
    for migration in migrations():
        for prefix, family in ENGINE_CLAUSE.findall(migration.read_text()):
            assert prefix == TOPOLOGY_MARKER, (
                f"{migration.name}: ENGINE = {prefix}{family} is fixed to one topology. "
                f"Spell it ENGINE = {TOPOLOGY_MARKER}{family} and the deploy renders "
                f"the replicated family where the warehouse replicates."
            )


def test_the_marker_is_only_ever_an_engine_prefix() -> None:
    """The deploy expands the marker wherever it appears, so a stray one would
    land `Replicated` in the middle of a column name or a string literal."""
    for migration in migrations():
        text = migration.read_text()
        rendered = ENGINE_CLAUSE.sub("", text)

        assert TOPOLOGY_MARKER not in rendered, f"{migration.name}: {TOPOLOGY_MARKER} appears outside an ENGINE clause"
