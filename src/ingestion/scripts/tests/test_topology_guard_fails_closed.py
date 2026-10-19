"""A topology check that could not run must stop the deploy, not pass it.

Every probe asks the warehouse a question, and each answer has a reading that
means "go ahead": no such database yet, no replicated directory needed. A probe
whose query never reached the server produces the same empty answer, so a guard
that ignores the failure waves through the install it exists to refuse — and
`CREATE DATABASE IF NOT EXISTS` then hides the wrong engine for good.

The transport is stubbed rather than pointed at a dead port: a refused
connection is one failure mode, and what must hold is that any unanswered probe
stops the deploy, whatever caused it.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
TOPOLOGY_LIB = SCRIPTS / "lib" / "ch-topology.sh"

#: A transport that answers nothing.
NOTHING_ANSWERS = "_ch_http_query() { return 7; }"

#: A transport that answers every probe but the engine lookup, so the preflight
#: reaches that lookup instead of stopping at the one before it.
ONLY_THE_ENGINE_LOOKUP_FAILS = """
_ch_http_query() {
  local sql
  sql="$(cat)"
  case "${sql}" in
    *system.databases*) return 7 ;;
    *) printf '1\\n' ;;
  esac
}
"""

REACHED_THE_END = "GUARD PASSED"


def run(body: str, *, transport: str = NOTHING_ANSWERS) -> subprocess.CompletedProcess[str]:
    """Run `body` against the real library under create-databases.sh's own
    shell options, with the HTTP transport replaced."""
    script = f"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
export CLICKHOUSE_CLUSTER_MODE=true CLICKHOUSE_CLUSTER_NAME=insight_cluster
source "$TOPOLOGY_LIB"
{transport}
{body}
echo '{REACHED_THE_END}'
"""

    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={"TOPOLOGY_LIB": str(TOPOLOGY_LIB), "PATH": "/usr/bin:/bin"},
        check=False,
    )


def test_the_preflight_refuses_a_warehouse_whose_engines_it_could_not_read() -> None:
    """Keeper and the access backend answer; only the engine lookup fails. The
    deploy must still stop — an unread database is not an absent one."""
    result = run("ch_assert_topology insight silver", transport=ONLY_THE_ENGINE_LOOKUP_FAILS)

    assert REACHED_THE_END not in result.stdout, (
        "the preflight passed over a database it never read, so the deploy would "
        f"create the rest with an unverified engine: {result.stdout!r}"
    )
    assert result.returncode != 0, result


def test_each_probe_names_the_question_it_could_not_answer() -> None:
    """`set -e` on a bare assignment stops the deploy with no output at all,
    which tells an operator nothing about which check failed or why."""
    for probe, table in (
        ("_ch_require_replicated_access_storage", "system.user_directories"),
        ("_ch_require_replicated_databases insight", "system.databases"),
    ):
        result = run(probe)

        assert result.returncode != 0, f"{probe} passed without reading {table}: {result}"
        assert table in result.stderr, f"{probe} failed without naming {table}: {result.stderr!r}"


def test_an_unreadable_database_is_not_read_as_an_absent_one() -> None:
    """The empty answer is the trap: it is also what a database that does not
    exist yet returns, and that reading lets the deploy carry on."""
    result = run('engine="$(_ch_database_engine_of silver)"; echo "engine=[${engine}]"')

    assert "engine=[]" not in result.stdout, (
        f"a failed lookup answered 'no such database', which every caller treats as nothing to check: {result.stdout!r}"
    )
    assert result.returncode != 0, result


def test_a_standalone_deploy_also_stops_on_an_unreadable_database() -> None:
    """The flag-off direction reads the same column for the opposite verdict,
    and silence there lets standalone DDL into a replicated warehouse."""
    result = run("CLICKHOUSE_CLUSTER_MODE=false ch_assert_topology insight silver")

    assert REACHED_THE_END not in result.stdout, result.stdout
    assert result.returncode != 0, result
