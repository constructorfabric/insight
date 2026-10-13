"""The catalogue first-sync heal builds the engine the topology needs, and an
unreadable probe stops it before anything is created or swapped.

The heal runs against a stub of `_ch_http_query`: each statement it sends is
logged, and the answers are scripted. Nothing here needs a ClickHouse — what is
asserted is which statements the heal is willing to send.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]

#: Answers by statement: the first pattern the statement contains wins. A pattern
#: answered with FAIL makes the stubbed HTTP call fail.
FAIL = "<fail>"

STUB = r"""
set -euo pipefail
source "$SCRIPTS/lib/ch-exec.sh"
source "$SCRIPTS/lib/heal-jira-catalogue-first-seen.sh"
_ch_http_query() {
  local sql
  sql="$(cat)"
  printf '%s\n;;\n' "$sql" >> "$LOG"
  case "$sql" in
    *"name='jira__catalogue_first_seen'"*) echo 1 ;;
    *"AND engine = "*) echo "$CONVERGED" ;;
    *"name = 'jira_fields'"*) [[ "$BRONZE" == "<fail>" ]] && return 1; echo "$BRONZE" ;;
    *"HAVING uniqExact"*) echo "$SHARED" ;;
    *"uniqExact(insight_source_id)"*) echo 2 ;;
    *) ;;
  esac
}
heal_jira_catalogue_first_seen
"""


@dataclass(frozen=True)
class Run:
    returncode: int
    statements: list[str]


def _heal(
    tmp_path: Path, *, cluster_mode: str = "false", converged: str = "0", bronze: str = "1", shared: str = "0"
) -> Run:
    log = tmp_path / "statements.log"
    log.touch()
    result = subprocess.run(
        ["bash", "-c", STUB],
        env={
            "PATH": "/usr/bin:/bin",
            "SCRIPTS": str(SCRIPTS),
            "LOG": str(log),
            "CONVERGED": converged,
            "BRONZE": bronze,
            "SHARED": shared,
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
    statements = [s.strip() for s in log.read_text().split("\n;;\n") if s.strip()]
    return Run(result.returncode, statements)


def _sent(run: Run, verb: str) -> list[str]:
    return [s for s in run.statements if s.lstrip().upper().startswith(verb)]


@pytest.mark.parametrize(
    ("cluster_mode", "engine"),
    [
        ("false", "ReplacingMergeTree"),
        ("true", "ReplicatedReplacingMergeTree"),
        (" Yes ", "ReplicatedReplacingMergeTree"),
    ],
)
def test_the_rebuilt_table_takes_the_engine_the_topology_needs(tmp_path: Path, cluster_mode: str, engine: str) -> None:
    run = _heal(tmp_path, cluster_mode=cluster_mode)

    creates = _sent(run, "CREATE TABLE")
    assert run.returncode == 0, f"cluster_mode={cluster_mode!r}: {run.statements}"
    assert len(creates) == 1, creates
    assert f"ENGINE = {engine}(_version)" in creates[0], f"cluster_mode={cluster_mode!r}: {creates[0]}"
    assert _sent(run, "EXCHANGE TABLES"), "the rebuilt table must be swapped in"


@pytest.mark.parametrize("cluster_mode", ["false", "true"])
def test_a_table_already_on_the_needed_engine_is_left_alone(tmp_path: Path, cluster_mode: str) -> None:
    run = _heal(tmp_path, cluster_mode=cluster_mode, converged="1")

    engine_probe = next(s for s in run.statements if "AND engine = " in s)
    expected = "ReplicatedReplacingMergeTree" if cluster_mode == "true" else "ReplacingMergeTree"
    assert run.returncode == 0
    assert f"engine = '{expected}'" in engine_probe, engine_probe
    assert not _sent(run, "CREATE TABLE") and not _sent(run, "EXCHANGE TABLES"), run.statements


@pytest.mark.parametrize("bronze", [FAIL, "not a number"], ids=["probe fails", "probe unreadable"])
def test_an_unreadable_bronze_probe_keeps_the_original_table(tmp_path: Path, bronze: str) -> None:
    run = _heal(tmp_path, bronze=bronze)

    assert run.returncode != 0, "the heal must stop rather than copy with no tenant"
    for verb in ("CREATE TABLE", "INSERT INTO", "EXCHANGE TABLES", "DROP TABLE"):
        assert not _sent(run, verb), f"{bronze!r}: sent {verb} after a failed probe"


def test_absent_bronze_copies_every_source_with_an_empty_tenant(tmp_path: Path) -> None:
    run = _heal(tmp_path, bronze="0")

    inserts = _sent(run, "INSERT INTO")
    assert run.returncode == 0, run.statements
    assert len(inserts) == 1 and "bronze_jira.jira_fields" not in inserts[0], inserts


def test_a_source_bronze_holds_under_several_tenants_keeps_the_original_table(tmp_path: Path) -> None:
    run = _heal(tmp_path, shared="1")

    assert run.returncode != 0, "the heal must refuse to copy one boundary to several tenants"
    for verb in ("CREATE TABLE", "INSERT INTO", "EXCHANGE TABLES", "DROP TABLE"):
        assert not _sent(run, verb), f"sent {verb} for an ambiguous source"
