"""A clustered deploy is refused unless the server can serve the cluster it names.

Four probes beyond Keeper and the access backend: the configured cluster exists
in `system.clusters`, both the `{shard}` and `{replica}` macros are defined,
every one of our Replicated databases has joined as many replicas as the
cluster has, and the flag never runs without a name. Each failure names the
remedy. The transport is stubbed: what each probe asks is answered from a
script, so a single wrong answer is what the guard must refuse.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
TOPOLOGY_LIB = SCRIPTS / "lib" / "ch-topology.sh"
CREATE_DATABASES = SCRIPTS / "create-databases.sh"

CLUSTER = "insight_cluster"
REACHED_THE_END = "GUARD PASSED"

#: A transport whose answers come from the environment: TARGET replicas in the
#: configured cluster, LONELY_JOINED replicas registered in Keeper by a
#: Replicated database named `lonely` (no other database exists anywhere),
#: MACROS defined, QUEUED hosts with its CREATE DATABASE still pending. Every
#: statement is appended to SQL_LOG.
SCRIPTED = """
_ch_http_query() {
  local sql
  sql="$(cat)"
  printf '%s\\n' "${sql}" >> "${SQL_LOG}"
  case "${sql}" in
    *system.clusters*) printf '%s\\n' "${TARGET}" ;;
    *system.macros*) printf '%s\\n' "${MACROS}" ;;
    *system.distributed_ddl_queue*) printf '%s\\n' "${QUEUED}" ;;
    *system.zookeeper*"/lonely/replicas"*) printf '%s\\n' "${LONELY_JOINED}" ;;
    *system.zookeeper*"/replicas"*) printf '0\\n' ;;
    *engine_full*"'lonely'"*) printf '/clickhouse/databases/lonely\\n' ;;
    *engine_full*) printf '\\n' ;;
    *system.databases*"'lonely'"*) printf 'Replicated\\n' ;;
    *system.databases*) printf '\\n' ;;
    *) printf '1\\n' ;;
  esac
}
"""


def run(
    body: str,
    *,
    cluster_name: str = CLUSTER,
    target: str = "2",
    lonely_joined: str = "2",
    macros: str = "2",
    queued: str = "0",
    log: Path | None = None,
) -> subprocess.CompletedProcess[str]:
    script = f"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
export CLICKHOUSE_CLUSTER_MODE=true CLICKHOUSE_CLUSTER_NAME='{cluster_name}'
source "$TOPOLOGY_LIB"
{SCRIPTED}
{body}
echo '{REACHED_THE_END}'
"""

    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={
            "TOPOLOGY_LIB": str(TOPOLOGY_LIB),
            "PATH": "/usr/bin:/bin",
            "TARGET": target,
            "LONELY_JOINED": lonely_joined,
            "MACROS": macros,
            "QUEUED": queued,
            "SQL_LOG": str(log or "/dev/null"),
        },
        check=False,
    )


def test_a_healthy_cluster_passes() -> None:
    result = run("ch_assert_topology insight lonely")

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout


def test_a_cluster_name_the_server_does_not_know_is_refused_before_any_ddl(tmp_path: Path) -> None:
    log = tmp_path / "sql.log"
    result = run(
        f'CLICKHOUSE_DATABASE=insight source "{CREATE_DATABASES}"', target="0", cluster_name="insihgt", log=log
    )

    assert result.returncode != 0, result
    assert "insihgt" in result.stderr and "system.clusters" in result.stderr, result.stderr
    assert "CREATE DATABASE" not in log.read_text(), "DDL was sent against a cluster the server does not have"


def test_a_server_without_both_macros_is_refused() -> None:
    result = run("ch_assert_topology insight", macros="1")

    assert result.returncode != 0, result
    assert "{shard}" in result.stderr and "{replica}" in result.stderr, result.stderr


def test_a_replicated_database_a_replica_never_joined_names_itself() -> None:
    """One replica short and nothing queued for it: the replica ran no CREATE
    DATABASE and will not catch up on its own."""
    result = run("ch_assert_topology insight lonely", lonely_joined="1")

    assert result.returncode != 0, result
    assert "'lonely'" in result.stderr, result.stderr
    assert "1 replica(s)" in result.stderr and "has 2" in result.stderr, result.stderr


def test_the_lag_is_read_from_keeper_not_from_the_replica_that_took_the_probe() -> None:
    """Through a load balancer the probe may land on the replica that lacks the
    database; Keeper's registration list is the same from every node."""
    result = run("ch_assert_topology insight lonely", lonely_joined="1")

    assert any("system.zookeeper" in line and "/lonely/replicas" in line for line in result.stderr.splitlines()) or (
        "/clickhouse/databases/lonely/replicas" in result.stderr
    ), result.stderr


def test_a_database_still_queued_for_an_inactive_replica_is_a_note_not_a_refusal() -> None:
    """A deploy that ran while a replica was down left its CREATE DATABASE in
    the Keeper queue; the replica replays it on return, and the deploy proceeds."""
    result = run("ch_assert_topology insight lonely", lonely_joined="1", queued="1")

    assert result.returncode == 0, result.stderr
    assert "lonely: on 1 of 2 replicas" in result.stdout, result.stdout


def test_a_database_short_by_more_than_the_queue_explains_is_refused() -> None:
    result = run("ch_assert_topology insight lonely", target="3", lonely_joined="1", queued="1")

    assert result.returncode != 0, result
    assert "'lonely'" in result.stderr, result.stderr


@pytest.mark.parametrize("body", ["ch_assert_topology insight", 'clause="$(ch_on_cluster)"'])
def test_the_flag_without_a_name_is_refused_at_shell_level(body: str) -> None:
    """The chart refuses the pair; a caller outside it must not get a
    node-local Replicated database out of the silence."""
    result = run(body, cluster_name="")

    assert result.returncode != 0, result
    assert "clusterName is empty" in result.stderr, result.stderr
    assert REACHED_THE_END not in result.stdout


def test_the_cluster_clause_marker_renders_per_topology() -> None:
    sql = "DROP TABLE IF EXISTS bronze_jira._boards /*@@ON_CLUSTER@@*/; ENGINE = /*@@REPLICATED@@*/MergeTree"

    clustered = run(f"printf '%s' '{sql}' | ch_render_topology; echo")
    standalone = run(f"CLICKHOUSE_CLUSTER_MODE=false; printf '%s' '{sql}' | ch_render_topology; echo")

    assert f"bronze_jira._boards ON CLUSTER {CLUSTER};" in clustered.stdout, clustered
    assert "ENGINE = ReplicatedMergeTree" in clustered.stdout, clustered
    assert "bronze_jira._boards ;" in standalone.stdout, standalone
    assert "ENGINE = MergeTree" in standalone.stdout, standalone


def test_a_database_named_like_the_cluster_is_not_counted_as_hosts(tmp_path: Path) -> None:
    """A Replicated database lists itself in system.clusters under its own
    name, one row per replica with `database_replica_name` set. The product
    database is `insight`; a cluster called `insight` would count those rows
    as hosts and refuse every database as incomplete."""
    log = tmp_path / "sql.log"
    result = run("_ch_require_cluster_name && _ch_require_complete_databases lonely", log=log)
    assert result.returncode == 0, result.stderr
    host_counts = [line for line in log.read_text().splitlines() if "system.clusters" in line]
    assert host_counts, log.read_text()
    assert all("database_replica_name = ''" in line for line in host_counts), host_counts
