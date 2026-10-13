"""The migration engine heal rebuilds a plain table on a clustered install from every
replica, and sends nothing it cannot verify.

The heal runs against a stub of `_ch_http_query`: each statement it sends is logged,
and the answers are scripted. Nothing here needs a ClickHouse — what is asserted is
which statements the heal is willing to send.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]

#: An answer that makes the stubbed HTTP call fail.
FAIL = "<fail>"

STUB = r"""
set -euo pipefail
source "$SCRIPTS/lib/ch-exec.sh"
source "$SCRIPTS/lib/heal-migration-engines.sh"
answer() { [[ "$1" == "<fail>" ]] && return 1; echo "$1"; }
_ch_http_query() {
  local sql
  sql="$(cat)"
  printf '%s\n;;\n' "$sql" >> "$LOG"
  case "$sql" in
    *"SELECT count() FROM system.tables"*) echo 1 ;;
    *"SELECT engine FROM"*) answer "$ENGINE" ;;
    *"SELECT engine_full FROM"*) echo "ReplacingMergeTree(_version) ORDER BY unique_key" ;;
    *"SELECT sorting_key FROM"*) echo "unique_key" ;;
    *"FROM clusterAllReplicas("*) answer "$BEFORE" ;;
    *"FROM silver.class_ai_credit_usage__replicated "*) answer "$COPIED" ;;
    *"FROM silver.class_ai_credit_usage "*) answer "$LIVE" ;;
    *) ;;
  esac
}
heal_migration_engine silver.class_ai_credit_usage
"""


@dataclass(frozen=True)
class Run:
    returncode: int
    statements: list[str]


def _heal(
    tmp_path: Path,
    *,
    cluster_mode: str = "true",
    cluster_name: str = "warehouse",
    engine: str = "ReplacingMergeTree",
    before: str = "60",
    copied: str = "60",
    live: str = "60",
) -> Run:
    log = tmp_path / "statements.log"
    log.touch()
    result = subprocess.run(
        ["bash", "-c", STUB],
        env={
            "PATH": "/usr/bin:/bin",
            "SCRIPTS": str(SCRIPTS),
            "LOG": str(log),
            "ENGINE": engine,
            "BEFORE": before,
            "COPIED": copied,
            "LIVE": live,
            "CLICKHOUSE_URL": "http://clickhouse.invalid:8123",
            "CLICKHOUSE_USER": "test",
            "CLICKHOUSE_PASSWORD": "test",
            "CLICKHOUSE_CLUSTER_MODE": cluster_mode,
            "CLICKHOUSE_CLUSTER_NAME": cluster_name,
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


WRITES = ("CREATE TABLE", "INSERT INTO", "EXCHANGE TABLES", "DROP TABLE")


def test_a_plain_table_on_a_cluster_is_rebuilt_replicated_from_every_replica(tmp_path: Path) -> None:
    run = _heal(tmp_path)

    creates = _sent(run, "CREATE TABLE")
    inserts = _sent(run, "INSERT INTO")
    assert run.returncode == 0, run.statements
    expected_create = (
        "CREATE TABLE silver.class_ai_credit_usage__replicated AS silver.class_ai_credit_usage "
        "ENGINE = ReplicatedReplacingMergeTree(_version) ORDER BY unique_key"
    )
    assert creates == [expected_create]
    assert len(inserts) == 1 and "clusterAllReplicas('warehouse', silver.class_ai_credit_usage)" in inserts[0], inserts
    assert _sent(run, "EXCHANGE TABLES") == [
        "EXCHANGE TABLES silver.class_ai_credit_usage AND silver.class_ai_credit_usage__replicated"
    ]


@pytest.mark.parametrize(
    ("cluster_mode", "engine"),
    [
        ("false", "ReplacingMergeTree"),
        ("true", "ReplicatedReplacingMergeTree"),
        ("false", "ReplicatedReplacingMergeTree"),
    ],
)
def test_a_table_already_on_an_engine_that_needs_no_rebuild_is_left_alone(
    tmp_path: Path, cluster_mode: str, engine: str
) -> None:
    run = _heal(tmp_path, cluster_mode=cluster_mode, engine=engine)

    assert run.returncode == 0, run.statements
    for verb in WRITES:
        assert not _sent(run, verb), f"cluster_mode={cluster_mode}, engine={engine}: sent {verb}"


@pytest.mark.parametrize(
    "overrides",
    [{"engine": FAIL}, {"before": FAIL}, {"before": "not a number"}, {"cluster_name": " "}],
    ids=["engine probe fails", "count fails", "count unreadable", "no cluster name"],
)
def test_an_unreadable_probe_keeps_the_original_table(tmp_path: Path, overrides: dict[str, str]) -> None:
    run = _heal(tmp_path, **overrides)

    assert run.returncode != 0, f"should stop: {overrides}"
    for verb in WRITES:
        assert not _sent(run, verb), f"{overrides}: sent {verb} before anything was counted"


@pytest.mark.parametrize("copied", ["59", FAIL])
def test_a_short_copy_is_dropped_and_never_swapped_in(tmp_path: Path, copied: str) -> None:
    run = _heal(tmp_path, copied=copied)

    assert run.returncode != 0, f"should stop on copied={copied!r}"
    assert not _sent(run, "EXCHANGE TABLES"), run.statements
    if copied != FAIL:
        assert _sent(run, "DROP TABLE")[-1] == "DROP TABLE IF EXISTS silver.class_ai_credit_usage__replicated"


def test_a_short_table_after_the_swap_is_swapped_back_and_the_original_kept(tmp_path: Path) -> None:
    run = _heal(tmp_path, live="59")

    exchanges = _sent(run, "EXCHANGE TABLES")
    drops = _sent(run, "DROP TABLE")
    assert run.returncode != 0
    assert len(exchanges) == 2, "the original must be swapped back"
    assert drops == ["DROP TABLE IF EXISTS silver.class_ai_credit_usage__replicated"], (
        "only the stale copy before the rebuild may be dropped"
    )
