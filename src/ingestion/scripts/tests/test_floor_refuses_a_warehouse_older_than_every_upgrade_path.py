"""A warehouse below the floor stops the deploy before any migration or heal.

The deploy hook keeps no ledger: a heal every supported upgrade path has
already run is retired, and the shape it converged becomes a floor fact in
`lib/warehouse-floor.sh`. A warehouse still holding the older shape skipped
those releases, and nothing in this hook would converge it any more — so the
hook refuses it by name, before it touches anything, instead of leaving the
next dbt run to fail on it.

The guard runs against a stub of `_ch_http_query`: the statement it sends is
logged, and the answer is scripted.
"""

from __future__ import annotations

import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
HOOK = SCRIPTS / "apply-ch-migrations.sh"

GUARD = "ch_assert_floor"
REACHED_THE_END = "HOOK CONTINUED"
FAIL = "<fail>"

STUB = rf"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
source "$SCRIPTS/lib/ch-exec.sh"
source "$SCRIPTS/lib/warehouse-floor.sh"
_ch_http_query() {{
  cat >> "$LOG"
  [[ "$ANSWER" == "{FAIL}" ]] && return 22
  printf '%b' "$ANSWER"
}}
{GUARD}
echo '{REACHED_THE_END}'
"""


def run(answer: str, tmp_path: Path) -> tuple[subprocess.CompletedProcess[str], str]:
    log = tmp_path / "statements.log"
    log.touch()
    result = subprocess.run(
        ["bash", "-c", STUB],
        capture_output=True,
        text=True,
        env={"PATH": "/usr/bin:/bin", "SCRIPTS": str(SCRIPTS), "LOG": str(log), "ANSWER": answer},
        check=False,
    )
    return result, log.read_text()


def test_a_warehouse_at_the_floor_passes(tmp_path: Path) -> None:
    result, _ = run("", tmp_path)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout


def test_a_warehouse_below_the_floor_is_refused_by_name(tmp_path: Path) -> None:
    below = "staging.cursor__ai_dev_usage.tool_label: left the contract, still stands\\n"
    result, _ = run(below, tmp_path)

    assert result.returncode != 0, result
    assert REACHED_THE_END not in result.stdout, "the hook carried on over a warehouse below the floor"
    assert "staging.cursor__ai_dev_usage.tool_label" in result.stderr, result.stderr
    assert "0.7.66" in result.stderr and "0.6.126" in result.stderr, "the refusal must name the releases to go through"


def test_a_probe_that_did_not_answer_is_not_read_as_at_the_floor(tmp_path: Path) -> None:
    result, _ = run(FAIL, tmp_path)

    assert result.returncode != 0, result
    assert REACHED_THE_END not in result.stdout, "an unanswered probe was read as a warehouse at the floor"
    assert "could not read" in result.stderr, result.stderr


@pytest.mark.parametrize(
    "fact",
    [
        "('staging', 'cursor__ai_dev_usage', 'tool_label', 'absent')",
        "('staging', 'github__commits', 'patch_id', 'present')",
        "('staging', 'jira__availability_events', 'event_kind', 'string')",
        "('staging', 'gitlab__pull_requests', 'lines_added', 'nullable')",
        "('bronze_jira', 'jira_comments', 'jira_id', 'present')",
        "('bronze_jira', 'jira_issue', 'jira_id', 'string')",
        "('staging', 'bitbucket_cloud__commits', 'committer_date', 'present')",
    ],
)
def test_the_probe_states_each_kind_of_fact(fact: str, tmp_path: Path) -> None:
    _, sent = run("", tmp_path)

    assert fact in sent, f"the floor probe should carry {fact}"


def test_the_guard_runs_before_any_migration_or_heal() -> None:
    """A heal or migration applied first would change the very shapes the guard
    reads, and a refused deploy must leave the warehouse as it found it."""
    lines = [line for line in HOOK.read_text().splitlines() if not line.strip().startswith("#")]
    text = "\n".join(lines)
    called = [line.strip() for line in lines if line.strip().startswith(GUARD)]

    assert called, f"{HOOK.name} never checks the floor"
    assert all("||" not in call and "if " not in call for call in called), f"the guard must stop the hook: {called}"
    assert text.index(GUARD) < text.index('migrations"/*.sql'), "a migration runs before the floor is checked"
    assert text.index(GUARD) < text.index("run_ch"), "a heal runs before the floor is checked"
