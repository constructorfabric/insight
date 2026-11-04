"""A standalone database is converted into what cluster mode creates.

The script renames the database aside, recreates it `ON CLUSTER` with the
engine `create-databases.sh` renders (none for a bronze database, which stays
Atomic), and re-issues every relation's own CREATE with its engine prefixed
`Replicated` — the cluster clause on each only in the bronze database — under
a staging name, copies the rows a reader sees, compares the counts and renames
the copy into place. A relation it cannot carry over is refused before
anything is renamed; a database already converted is left alone. The
transport is stubbed: every statement is recorded, and the answers come from
the environment.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
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
#: (name<TAB>engine lines), the converted database holds the relations named
#: in EXISTING, a table counts SOURCE_ROWS / COPY_ROWS (raw) and SOURCE_FINAL /
#: COPY_FINAL (with FINAL) and hashes its rows to SOURCE_CONTENT / COPY_CONTENT by
#: the side it stands on, and SHOW CREATE answers with the relation it was
#: asked for.
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
    *"system.tables WHERE database"*"AND name ="*)
      named="$(printf '%s' "${sql}" | sed -E "s/.*name = '([^']+)'.*/\\1/")"
      if [[ " ${EXISTING} " == *" ${named} "* ]]; then printf '1\\n'; else printf '0\\n'; fi ;;
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
    "SELECT count() FROM "*)
      side=COPY
      [[ "${sql}" == *"FROM \\`standalone_"* ]] && side=SOURCE
      if [[ "${sql}" == *" FINAL" ]]; then side+=_FINAL; else side+=_ROWS; fi
      printf '%s\\n' "${!side}" ;;
    "SELECT sum(cityHash64(tuple(*))) FROM "*)
      if [[ "${sql}" == *"FROM \\`standalone_"* ]]; then printf '%s\\n' "${SOURCE_CONTENT}"; else printf '%s\\n' "${COPY_CONTENT}"; fi ;;
    *) printf '\\n' ;;
  esac
}
"""


@dataclass(frozen=True)
class Counts:
    source_rows: int = 3
    source_final: int = 3
    copy_rows: int = 3
    copy_final: int = 3
    source_content: int = 11
    copy_content: int = 11


#: A replacing table whose parts still hold rows sharing a key: the copy has
#: collapsed some of them, and a reader sees the same rows on both sides.
UNMERGED = Counts(source_rows=7, source_final=3, copy_rows=5, copy_final=3)


def reads_the_source(statement: str) -> bool:
    return statement.startswith(("SELECT ", "SHOW CREATE ", "RENAME DATABASE ", "INSERT INTO `bronze_github`"))


def run(
    database: str,
    *,
    log: Path,
    engine: str = "Atomic",
    relations: str = "t\\tReplacingMergeTree",
    listing_fails_after_rename: bool = False,
    replicas_unreachable: bool = False,
    existing: str = "",
    counts: Counts | None = None,
) -> subprocess.CompletedProcess[str]:
    counts = counts or Counts()
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
            "EXISTING": existing,
            "SOURCE_ROWS": str(counts.source_rows),
            "SOURCE_FINAL": str(counts.source_final),
            "COPY_ROWS": str(counts.copy_rows),
            "COPY_FINAL": str(counts.copy_final),
            "SOURCE_CONTENT": str(counts.source_content),
            "COPY_CONTENT": str(counts.copy_content),
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
        "CREATE TABLE silver.`t__converting`\n(\n    `a` UInt8\n)\n"
        "ENGINE = ReplicatedReplacingMergeTree(_version)\nORDER BY a"
    ) in sql
    assert "INSERT INTO `silver`.`t__converting` SELECT * FROM `standalone_silver`.`t` FINAL\n" in sql
    assert "RENAME TABLE `silver`.`t__converting` TO `silver`.`t`\n" in sql
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
    assert f"CREATE TABLE bronze_github.`t__converting` ON CLUSTER {CLUSTER}\n(" in sql
    assert "ENGINE = ReplicatedReplacingMergeTree(_version)" in sql
    assert f"RENAME TABLE `bronze_github`.`t__converting` TO `bronze_github`.`t` ON CLUSTER {CLUSTER}" in sql


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


@pytest.mark.parametrize(
    ("engine", "counted"),
    [
        ("ReplacingMergeTree", " FINAL"),
        ("VersionedCollapsingMergeTree", " FINAL"),
        ("AggregatingMergeTree", " FINAL"),
        ("MergeTree", ""),
    ],
)
def test_the_counts_compared_are_the_rows_a_reader_of_the_engine_sees(
    tmp_path: Path, engine: str, counted: str
) -> None:
    log = tmp_path / "sql.log"
    result = run("bronze_github", relations=f"t\\t{engine}", log=log)

    assert result.returncode == 0, result.stderr
    sql = log.read_text()
    for side in ("`standalone_bronze_github`.`t`", "`bronze_github`.`t__converting`"):
        assert f"SELECT count() FROM {side}{counted}\n" in sql, f"{engine} should count {side}{counted}: {sql}"
    copied = "INSERT INTO `bronze_github`.`t__converting` SELECT * FROM `standalone_bronze_github`.`t`"
    assert f"{copied}{counted}\n" in sql, f"{engine} should copy what a reader sees: {sql}"


def test_a_replacing_table_with_unmerged_duplicates_converts(tmp_path: Path) -> None:
    log = tmp_path / "sql.log"
    result = run("bronze_github", log=log, counts=UNMERGED)

    assert result.returncode == 0, result.stderr
    assert "bronze_github.t: 3 row(s)" in result.stdout
    assert "RENAME TABLE `bronze_github`.`t__converting`" in log.read_text()


@pytest.mark.parametrize(
    ("engine", "lost"),
    [
        ("ReplacingMergeTree", Counts(source_rows=7, source_final=3, copy_rows=5, copy_final=2)),
        # FINAL would hide it: a plain table is held to its raw rows.
        ("MergeTree", Counts(source_rows=7, source_final=3, copy_rows=6, copy_final=3)),
    ],
)
def test_a_copy_that_lost_rows_stops_before_it_takes_the_name_and_leaves_the_source(
    tmp_path: Path, engine: str, lost: Counts
) -> None:
    log = tmp_path / "sql.log"
    result = run("bronze_github", relations=f"t\\t{engine}", log=log, counts=lost)

    assert result.returncode != 0, f"{engine}: a lost row must stop the conversion"
    assert REACHED_THE_END not in result.stdout
    assert "bronze_github.t__converting holds" in result.stderr, result.stderr
    sql = log.read_text()
    assert "RENAME TABLE" not in sql, f"{engine}: a short copy must not take the table's name"
    writes = [line for line in sql.splitlines() if "standalone_bronze_github" in line and not reads_the_source(line)]
    assert writes == [], f"{engine}: the source is read, never written: {writes}"


@pytest.mark.parametrize(
    ("engine", "counts", "kept"),
    [
        ("ReplacingMergeTree", UNMERGED, True),
        ("ReplacingMergeTree", Counts(copy_content=12), False),
        ("ReplacingMergeTree", Counts(copy_final=2), False),
        ("MergeTree", Counts(copy_content=12), True),
        ("MergeTree", Counts(copy_rows=2), False),
    ],
)
def test_a_table_already_under_its_name_is_kept_only_when_it_shows_the_rows_the_source_shows(
    tmp_path: Path, engine: str, counts: Counts, kept: bool
) -> None:
    """A raw copy by an earlier version of the script can match FINAL's count but not its rows."""
    log = tmp_path / "sql.log"
    result = run("bronze_github", relations=f"t\\t{engine}", log=log, existing="t", counts=counts)

    assert (result.returncode == 0) is kept, f"{engine} {counts}: {result.stderr}"
    assert ("already copied" in result.stdout) is kept
    sql = log.read_text()
    assert "INSERT" not in sql
    assert ("cityHash64(tuple(*))" in sql) is (engine != "MergeTree" and counts.copy_final == counts.source_final)


def test_a_copy_left_under_its_staging_name_is_refused_by_that_name(tmp_path: Path) -> None:
    log = tmp_path / "sql.log"
    result = run("bronze_github", log=log, existing="t__converting")

    assert result.returncode != 0
    assert "bronze_github.t__converting is a copy of standalone_bronze_github.t that never" in result.stderr
    assert f"Drop it ON CLUSTER {CLUSTER} and re-run" in result.stderr, result.stderr
    assert "INSERT" not in log.read_text()
