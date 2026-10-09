"""`create-databases.sh` is the only place a ClickHouse database is created.

The cluster mechanism (epic #2010) rests on it: a `Replicated` database engine
belongs on the `CREATE DATABASE` statement, and a second creator that wins the
race on a fresh cluster leaves a plain database behind — silently, because every
statement in the deploy path is `IF NOT EXISTS`.

The rest of the deploy path — the numbered migrations, the connectors-ddl
snapshot, the dbt `on-run-start` hooks, the compose bootstrap, the chart — may
assume its database already stands.

Read with comments stripped: these files explain the rule in prose, and a
guard that matched its own documentation would fire on every mention.

Throwaway warehouses are exempt: a test rig that builds and drops its own
ClickHouse never runs on a cluster and creates whatever it needs.
"""

from __future__ import annotations

import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[4]
INGESTION = REPO_ROOT / "src/ingestion"

THE_SITE = INGESTION / "scripts/create-databases.sh"

#: Every deploy-path tree a `CREATE DATABASE` must not appear in, as (root, glob).
DEPLOY_PATH = (
    (INGESTION / "scripts/migrations", "*.sql"),
    (INGESTION / "scripts/connectors-ddl", "*.sql"),
    (INGESTION / "dbt/macros", "*.sql"),
    (REPO_ROOT / "deploy/compose", "*.sql"),
    (REPO_ROOT / "charts/insight/templates", "*.yaml"),
)

#: MariaDB has its own creator, its own init Job and no cluster fork.
EXEMPT = ("mariadb-init.sql", "mariadb-init-svcdbs-job.yaml")

CREATES_A_DATABASE = re.compile(r"\bCREATE DATABASE\b", re.IGNORECASE)

JINJA_COMMENT = re.compile(r"\{#.*?#\}", re.DOTALL)
HELM_COMMENT = re.compile(r"\{\{-?\s*/\*.*?\*/\s*-?\}\}", re.DOTALL)
LINE_COMMENT = re.compile(r"^\s*(--|#)")


def creating_lines(path: Path) -> list[str]:
    """Lines issuing a CREATE DATABASE, with every comment form stripped first."""
    body = HELM_COMMENT.sub("", JINJA_COMMENT.sub("", path.read_text(encoding="utf-8")))
    return [
        line.strip() for line in body.splitlines() if CREATES_A_DATABASE.search(line) and not LINE_COMMENT.match(line)
    ]


def test_no_deploy_path_file_creates_a_database() -> None:
    offenders = {
        str(path.relative_to(REPO_ROOT)): lines
        for root, pattern in DEPLOY_PATH
        for path in sorted(root.glob(pattern))
        if path.name not in EXEMPT and (lines := creating_lines(path))
    }
    assert not offenders, (
        "a deploy-path file creates a ClickHouse database; add it to the "
        f"DATABASES list in {THE_SITE.relative_to(REPO_ROOT)} instead: {offenders}"
    )


def test_the_site_itself_creates_databases() -> None:
    """A guard whose one exception stopped creating anything would pass empty."""
    assert creating_lines(THE_SITE), f"{THE_SITE} issues no CREATE DATABASE"


def test_the_inventory_itself_is_not_empty() -> None:
    """A glob that matches nothing would make the guard above pass by accident."""
    for root, pattern in DEPLOY_PATH:
        assert list(root.glob(pattern)), f"no {pattern} files found under {root}"
