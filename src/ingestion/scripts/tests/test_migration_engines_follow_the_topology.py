"""Every engine a numbered migration creates is the one its install's topology needs.

A clustered ClickHouse creates `Replicated*` where a single node creates the plain
family (epic #2010). Inside a Replicated database a plain engine replicates the DDL
but never the rows, so a literal family leaves each replica with rows of its own.
Migrations name their engine as `{{ch_engine:<family>}}`, which `ch_render_engines`
renders at deploy time — the counterpart of `insight_engine` for dbt models.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
MIGRATIONS = SCRIPTS / "migrations"

#: An `ENGINE =` that is not the topology's token.
ENGINE_LITERAL = re.compile(r"\bENGINE\s*=(?!\s*\{\{ch_engine:)", re.IGNORECASE)

#: A table a migration creates with the topology's engine.
TOPOLOGY_TABLE = re.compile(
    r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?`?([\w.]+?)`?\s*\([^;]*?ENGINE\s*=\s*\{\{ch_engine:",
    re.IGNORECASE | re.DOTALL,
)


def _sql(path: Path) -> str:
    return "\n".join(line for line in path.read_text().splitlines() if not line.strip().startswith("--"))


def _bash(script: str, *, cluster_mode: str = "false", stdin: str = "") -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", "-c", f'set -euo pipefail; source "$SCRIPTS/lib/ch-exec.sh"; {script}'],
        input=stdin,
        env={
            "PATH": "/usr/bin:/bin",
            "SCRIPTS": str(SCRIPTS),
            "CLICKHOUSE_URL": "http://clickhouse.invalid:8123",
            "CLICKHOUSE_USER": "test",
            "CLICKHOUSE_PASSWORD": "test",
            "CLICKHOUSE_CLUSTER_MODE": cluster_mode,
        },
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


def _render(sql: str, *, cluster_mode: str) -> subprocess.CompletedProcess[str]:
    return _bash("ch_render_engines", cluster_mode=cluster_mode, stdin=sql)


def test_a_migration_states_its_engine_through_the_token_rather_than_as_a_literal() -> None:
    literals = [
        f"{path.name}: {match.group(0)}"
        for path in sorted(MIGRATIONS.glob("*.sql"))
        for match in ENGINE_LITERAL.finditer(_sql(path))
    ]

    assert not literals, f"engine created without {{{{ch_engine:...}}}}: {literals}"


@pytest.mark.parametrize("migration", sorted(MIGRATIONS.glob("*.sql")), ids=lambda path: path.name)
@pytest.mark.parametrize(("cluster_mode", "prefix"), [("false", ""), ("true", "Replicated")])
def test_every_migration_renders_the_family_the_topology_needs(migration: Path, cluster_mode: str, prefix: str) -> None:
    source = migration.read_text()
    families = re.findall(r"\{\{ch_engine:(\w+)\}\}", source)

    rendered = _render(source, cluster_mode=cluster_mode)

    assert rendered.returncode == 0, f"{migration.name}: {rendered.stderr}"
    assert "{{" not in rendered.stdout, f"{migration.name}: token left unrendered"
    for family in families:
        assert f"ENGINE = {prefix}{family}" in rendered.stdout, f"{migration.name} under cluster_mode={cluster_mode}"
    if not prefix:
        assert "Replicated" not in rendered.stdout, f"{migration.name}: a single node cannot replicate"


@pytest.mark.parametrize("token", ["{{ch_engine:Log}}", "{{ch_engine:ReplicatedMergeTree}}", "{{ch_engine:}}"])
def test_a_token_naming_no_plain_merge_tree_family_fails_the_render(token: str) -> None:
    rendered = _render(f"CREATE TABLE t (k UInt8) ENGINE = {token} ORDER BY k;", cluster_mode="true")

    assert rendered.returncode != 0, f"should reject: {token!r}"


def test_every_table_a_migration_creates_is_healed_onto_the_topology_engine() -> None:
    """A table created before its migration used the token keeps its literal engine
    on every install that already holds it, unless the heal rebuilds it."""
    created = {match for path in sorted(MIGRATIONS.glob("*.sql")) for match in TOPOLOGY_TABLE.findall(_sql(path))}
    listed = _bash('source "$SCRIPTS/lib/heal-migration-engines.sh"; printf "%s\\n" "${MIGRATION_ENGINE_TABLES[@]}"')

    assert listed.returncode == 0, listed.stderr
    assert created, "no migration creates a table — the pattern found nothing to check"
    assert set(listed.stdout.split()) == created
