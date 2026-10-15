"""A retired database or table is dropped only while nothing in it can be data.

The deploy hook re-runs on every deploy against installs this repository never
sees, so "retired" is a statement about the code, not about what an operator
kept there. A count that could not be read, a relation that reports no count,
and a replica this node cannot see all have to read as "holds rows".

The transport is stubbed: every statement sent is recorded, and each probe gets
the answer the case names.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
RETIRED_LIB = SCRIPTS / "lib" / "retired-relations.sh"

REACHED_THE_END = "HEAL FINISHED"


def run(body: str, *, held: str, orphans: str = "", cluster: bool = False, tmp_path: Path) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    """Run `body` against the real library under apply-ch-migrations.sh's shell
    options. Probes for held rows answer `held`, the orphan listing answers
    `orphans`, anything else succeeds silently."""
    sent = tmp_path / "sent.sql"
    transport = f"""
_ch_http_query() {{
  local sql
  sql="$(cat)"
  printf '%s\\n' "${{sql}}" >> "{sent}"
  case "${{sql}}" in
    *"total_rows IS NULL"*) printf '%s\\n' '{held}' ;;
    *"inner_id"*) printf '%b' '{orphans}' ;;
  esac
}}
"""
    script = f"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
export CLICKHOUSE_CLUSTER_MODE={"true" if cluster else "false"} CLICKHOUSE_CLUSTER_NAME=insight_cluster
source "$RETIRED_LIB"
{transport}
{body}
echo '{REACHED_THE_END}'
"""
    result = subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={"RETIRED_LIB": str(RETIRED_LIB), "PATH": "/usr/bin:/bin"},
        check=False,
    )
    statements = sent.read_text().splitlines() if sent.exists() else []
    return result, statements


def drops(statements: list[str]) -> list[str]:
    return [s for s in statements if s.startswith("DROP ")]


CALLS = [
    ("drop_retired_database_if_empty bronze_figma", "DROP DATABASE IF EXISTS `bronze_figma`"),
    ("drop_retired_table_if_empty insight raw_data", "DROP TABLE IF EXISTS `insight`.`raw_data`"),
]


@pytest.mark.parametrize(("call", "statement"), CALLS)
def test_an_empty_retired_relation_is_dropped(call: str, statement: str, tmp_path: Path) -> None:
    result, sent = run(call, held="0", tmp_path=tmp_path)

    assert REACHED_THE_END in result.stdout, result
    assert drops(sent) == [statement], f"should drop exactly {statement!r}: {sent!r}"


@pytest.mark.parametrize(("call", "statement"), CALLS)
def test_a_retired_relation_that_may_hold_rows_is_left_in_place(call: str, statement: str, tmp_path: Path) -> None:
    result, sent = run(call, held="1", tmp_path=tmp_path)

    assert REACHED_THE_END in result.stdout, result
    assert drops(sent) == [], f"should keep a relation holding rows: {call!r} sent {sent!r}"
    assert "left in place" in result.stdout, result


@pytest.mark.parametrize(("call", "statement"), CALLS)
@pytest.mark.parametrize("answer", ["", "Code: 497. DB::Exception", "1.5"])
def test_an_unreadable_count_stops_the_deploy(call: str, statement: str, answer: str, tmp_path: Path) -> None:
    result, sent = run(call, held=answer, tmp_path=tmp_path)

    assert REACHED_THE_END not in result.stdout, f"should stop on count {answer!r}: {result.stdout!r}"
    assert result.returncode != 0, result
    assert drops(sent) == [], f"should send no DROP after count {answer!r}: {sent!r}"


@pytest.mark.parametrize(("call", "statement"), CALLS)
def test_a_clustered_warehouse_is_never_counted_from_one_replica(call: str, statement: str, tmp_path: Path) -> None:
    result, sent = run(call, held="0", cluster=True, tmp_path=tmp_path)

    assert REACHED_THE_END in result.stdout, result
    assert sent == [], f"should not even probe on a cluster: {call!r} sent {sent!r}"


def test_each_orphaned_inner_table_is_dropped_by_its_quoted_name(tmp_path: Path) -> None:
    orphans = ".inner_id.00000000-1111-2222-3333-444444444444\\n.inner_id.55555555-6666-7777-8888-999999999999\\n"
    result, sent = run("drop_orphaned_inner_tables insight", held="0", orphans=orphans, tmp_path=tmp_path)

    assert REACHED_THE_END in result.stdout, result
    assert drops(sent) == [
        "DROP TABLE IF EXISTS `insight`.`.inner_id.00000000-1111-2222-3333-444444444444`",
        "DROP TABLE IF EXISTS `insight`.`.inner_id.55555555-6666-7777-8888-999999999999`",
    ], sent


def test_the_orphan_listing_spares_live_detached_and_new_views(tmp_path: Path) -> None:
    _, sent = run("drop_orphaned_inner_tables insight", held="0", tmp_path=tmp_path)

    listing = next(s for s in sent if "inner_id" in s)
    for guard in ("engine = 'MaterializedView'", "system.detached_tables", "INTERVAL 1 DAY", "substring(name, 11)"):
        assert guard in listing, f"orphan listing should carry {guard!r}: {listing!r}"


def test_a_failed_orphan_listing_stops_the_deploy(tmp_path: Path) -> None:
    script_body = "_ch_http_query() { return 7; }\ndrop_orphaned_inner_tables insight"
    result, sent = run(script_body, held="0", tmp_path=tmp_path)

    assert REACHED_THE_END not in result.stdout, result.stdout
    assert result.returncode != 0, result
