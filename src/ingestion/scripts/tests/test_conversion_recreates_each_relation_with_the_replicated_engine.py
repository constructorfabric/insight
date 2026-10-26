"""A standalone database is converted into what cluster mode creates.

The script renames the database aside, recreates it `ON CLUSTER` with the
engine `create-databases.sh` renders (none for a bronze database, which stays
Atomic), and re-issues every relation's own CREATE with its engine prefixed
`Replicated` — the cluster clause on each only in the bronze database — then
copies the rows and compares the counts. A relation it cannot carry over is
refused before anything is renamed; a database already converted is left
alone. The transport is stubbed: every statement is recorded, and the answers
come from the environment.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
TOPOLOGY_LIB = SCRIPTS / "lib" / "ch-topology.sh"
CONVERT = SCRIPTS / "convert-database-to-cluster.sh"

CLUSTER = "insight_cluster"
REACHED_THE_END = "CONVERSION RETURNED"

#: Answers by statement: the server is a healthy cluster, the database named
#: by the run carries DB_ENGINE until it is renamed, no `standalone_*`
#: database exists yet, every listing of relations is RELATIONS
#: (name<TAB>engine lines), every count is 3, and SHOW CREATE answers with
#: the relation it was asked for.
SCRIPTED = """
_ch_http_query() {
  local sql
  sql="$(cat)"
  printf '%s\\n' "${sql}" >> "${SQL_LOG}"
  case "${sql}" in
    "RENAME DATABASE "*) : > "${SQL_LOG}.renamed" ;;
    *"FORMAT TSV")
      if [[ -n "${LISTING_FAILS_AFTER_RENAME:-}" && -e "${SQL_LOG}.renamed" ]]; then return 7; fi
      printf '%b\\n' "${RELATIONS}" ;;
    *"system.databases WHERE name = 'standalone_"*) printf '\\n' ;;
    *clusterAllReplicas*system.databases*)
      if [[ -n "${REPLICAS_UNREACHABLE:-}" ]]; then return 7; fi
      printf '2\\n' ;;
    *system.databases*)
      if [[ -e "${SQL_LOG}.renamed" ]]; then printf '\\n'; else printf '%s\\n' "${DB_ENGINE}"; fi ;;
    *system.clusters*|*system.macros*) printf '2\\n' ;;
    *system.zookeeper*) printf '1\\n' ;;
    *"endsWith(engine, 'MergeTree')"*) printf '1\\n' ;;
    *"system.tables WHERE database"*"AND name ="*) printf '0\\n' ;;
    "SHOW CREATE TABLE "*)
      [[ "${sql}" =~ \\`([^\\`]+)\\`\\.\\`([^\\`]+)\\` ]]
      if [[ "${BASH_REMATCH[2]}" == v ]]; then
        # The query keeps the names the view was created under.
        printf 'CREATE VIEW %s.v\\n(\\n    `a` UInt8\\n)\\nAS SELECT a FROM %s.t\\n' \\
          "${BASH_REMATCH[1]}" "${BASH_REMATCH[1]#standalone_}"
      else
        printf 'CREATE TABLE %s.%s\\n(\\n    `a` UInt8\\n)\\nENGINE = ReplacingMergeTree(_version)\\nORDER BY a\\n' \\
          "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
      fi ;;
    "SELECT count() FROM "*) printf '3\\n' ;;
    *) printf '\\n' ;;
  esac
}
"""


def run(
    database: str,
    *,
    log: Path,
    engine: str = "Atomic",
    relations: str = "t\\tReplacingMergeTree",
    listing_fails_after_rename: bool = False,
    replicas_unreachable: bool = False,
) -> subprocess.CompletedProcess[str]:
    script = f"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
export CLICKHOUSE_CLUSTER_MODE=true CLICKHOUSE_CLUSTER_NAME='{CLUSTER}'
source "$TOPOLOGY_LIB"
{SCRIPTED}
source "$CONVERT" '{database}'
echo '{REACHED_THE_END}'
"""

    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={
            "TOPOLOGY_LIB": str(TOPOLOGY_LIB),
            "CONVERT": str(CONVERT),
            "PATH": "/usr/bin:/bin",
            "DB_ENGINE": engine,
            "RELATIONS": relations,
            "SQL_LOG": str(log),
            "LISTING_FAILS_AFTER_RENAME": "1" if listing_fails_after_rename else "",
            "REPLICAS_UNREACHABLE": "1" if replicas_unreachable else "",
        },
        check=False,
    )


def test_an_owned_database_becomes_replicated_and_its_relations_carry_no_clause(tmp_path: Path) -> None:
    log = tmp_path / "sql.log"
    result = run("silver", relations="t\\tReplacingMergeTree\\nv\\tView", log=log)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout
    sql = log.read_text()
    assert "RENAME DATABASE `silver` TO `standalone_silver`" in sql
    assert (
        f"CREATE DATABASE `silver` ON CLUSTER {CLUSTER} "
        "ENGINE = Replicated('/clickhouse/databases/silver', '{shard}', '{replica}')"
    ) in sql
    assert (
        "CREATE TABLE silver.t\n(\n    `a` UInt8\n)\nENGINE = ReplicatedReplacingMergeTree(_version)\nORDER BY a" in sql
    )
    assert "INSERT INTO `silver`.`t` SELECT * FROM `standalone_silver`.`t`" in sql
    assert "CREATE VIEW silver.v\n(\n    `a` UInt8\n)\nAS SELECT a FROM silver.t" in sql
    assert "silver.t ON CLUSTER" not in sql and "silver.v ON CLUSTER" not in sql
    assert "DROP" not in sql, "the standalone copy is the rollback and stays"
    assert "silver: 1 table(s) and 1 view(s) converted" in result.stdout


def test_a_bronze_database_stays_atomic_and_its_relations_carry_the_clause(tmp_path: Path) -> None:
    log = tmp_path / "sql.log"
    result = run("bronze_github", log=log)

    assert result.returncode == 0, result.stderr
    sql = log.read_text()
    assert f"CREATE DATABASE `bronze_github` ON CLUSTER {CLUSTER}\n" in sql
    assert "Replicated('/clickhouse/databases/bronze_github'" not in sql
    assert f"CREATE TABLE bronze_github.t ON CLUSTER {CLUSTER}\n(" in sql
    assert "ENGINE = ReplicatedReplacingMergeTree(_version)" in sql


@pytest.mark.parametrize(
    ("relations", "named"),
    [("t\\tReplacingMergeTree\\nd\\tDictionary", "d  Dictionary"), ("m\\tMaterializedView", "m  MaterializedView")],
)
def test_a_relation_the_conversion_cannot_carry_over_is_refused_before_the_rename(
    tmp_path: Path, relations: str, named: str
) -> None:
    log = tmp_path / "sql.log"
    result = run("silver", relations=relations, log=log)

    assert result.returncode != 0
    assert REACHED_THE_END not in result.stdout
    assert named in result.stderr, result.stderr
    sql = log.read_text()
    assert "RENAME" not in sql and "CREATE" not in sql, f"should move nothing: {sql}"


def test_a_listing_that_fails_after_the_rename_stops_the_conversion(tmp_path: Path) -> None:
    log = tmp_path / "sql.log"
    result = run("silver", log=log, listing_fails_after_rename=True)

    assert result.returncode != 0
    assert REACHED_THE_END not in result.stdout
    assert "converted" not in result.stdout, "an unread database must not be reported as converted"
    assert "INSERT" not in log.read_text()


def test_a_bronze_probe_that_cannot_reach_every_replica_stops_before_the_rename(tmp_path: Path) -> None:
    """A probe that failed must not read as "not converted yet"."""
    log = tmp_path / "sql.log"
    result = run("bronze_github", log=log, replicas_unreachable=True)

    assert result.returncode != 0
    assert "could not read system.databases on every replica" in result.stderr, result.stderr
    assert "RENAME" not in log.read_text(), "a database whose state is unknown must not be moved"


def test_a_database_already_replicated_is_left_alone(tmp_path: Path) -> None:
    log = tmp_path / "sql.log"
    result = run("silver", engine="Replicated", log=log)

    assert result.returncode == 0, result.stderr
    assert "nothing to do" in result.stdout
    sql = log.read_text()
    assert "RENAME" not in sql and "CREATE" not in sql and "INSERT" not in sql


def test_the_flag_off_is_refused_before_anything_is_read() -> None:
    result = subprocess.run(
        ["bash", "-c", 'CLICKHOUSE_CLUSTER_MODE=false "$CONVERT" silver'],
        capture_output=True,
        text=True,
        env={
            "CONVERT": str(CONVERT),
            "PATH": "/usr/bin:/bin",
            "CLICKHOUSE_URL": "http://127.0.0.1:1",
            "CLICKHOUSE_USER": "u",
            "CLICKHOUSE_PASSWORD": "p",
        },
        check=False,
    )

    assert result.returncode != 0
    assert "CLICKHOUSE_CLUSTER_MODE is off" in result.stderr, result.stderr
