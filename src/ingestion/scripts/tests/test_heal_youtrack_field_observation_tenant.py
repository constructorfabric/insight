"""The YouTrack field-observation tenant heal skips a table only when ClickHouse
says it is absent; a probe it cannot read stops the deploy.

The heal runs against a stub of `_ch_http_query`: each statement it sends is
logged and the answers are scripted, so no ClickHouse is needed.
"""

from __future__ import annotations

import subprocess
from dataclasses import dataclass
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
FAIL = "<fail>"

STUB = r"""
set -euo pipefail
source "$SCRIPTS/lib/ch-exec.sh"
source "$SCRIPTS/lib/heal-youtrack-field-observation-tenant.sh"
_answer() { [[ "$1" == "<fail>" ]] && return 1; printf '%s\n' "$1"; }
_ch_http_query() {
  local sql
  sql="$(cat)"
  printf '%s\n;;\n' "$sql" >> "$LOG"
  case "$sql" in
    *"name = 'youtrack__field_observations'"*) _answer "$STAGING" ;;
    *"name = 'youtrack_custom_fields'"*) _answer "$BRONZE" ;;
    *"groupUniqArray"*) echo "['0f6b3c2e-5d4a-4e7b-9c1d-2a3b4c5d6e7f']" ;;
    *) ;;
  esac
}
heal_youtrack_field_observation_tenant
"""


@dataclass(frozen=True)
class Run:
    returncode: int
    statements: list[str]

    def altered(self) -> list[str]:
        return [s for s in self.statements if s.lstrip().upper().startswith("ALTER")]


def _heal(tmp_path: Path, *, staging: str, bronze: str) -> Run:
    log = tmp_path / "statements.log"
    log.touch()
    result = subprocess.run(
        ["bash", "-c", STUB],
        env={
            "PATH": "/usr/bin:/bin:/opt/homebrew/bin",
            "SCRIPTS": str(SCRIPTS),
            "LOG": str(log),
            "STAGING": staging,
            "BRONZE": bronze,
            "CLICKHOUSE_URL": "http://clickhouse.invalid:8123",
            "CLICKHOUSE_USER": "test",
            "CLICKHOUSE_PASSWORD": "test",
        },
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )
    statements = [s.strip() for s in log.read_text().split("\n;;\n") if s.strip()]
    return Run(result.returncode, statements)


def test_the_column_is_added_and_backfilled_when_both_tables_exist(tmp_path: Path) -> None:
    run = _heal(tmp_path, staging="1", bronze="1")

    assert run.returncode == 0
    assert [s.split()[3] + " " + s.split()[4] for s in run.altered()] == [
        "ADD COLUMN",
        "MODIFY COLUMN",
        "UPDATE tenant_id",
    ]


@pytest.mark.parametrize(
    ("staging", "bronze", "altered"),
    [("0", "1", 0), ("1", "0", 2)],
    ids=["no observations table", "no YouTrack Bronze"],
)
def test_a_table_confirmed_absent_is_skipped(tmp_path: Path, staging: str, bronze: str, altered: int) -> None:
    run = _heal(tmp_path, staging=staging, bronze=bronze)

    assert run.returncode == 0
    assert len(run.altered()) == altered, f"should alter {altered} times: {run.altered()}"


@pytest.mark.parametrize(
    ("staging", "bronze", "altered"),
    [(FAIL, "1", 0), ("not a number", "1", 0), ("1", FAIL, 2)],
    ids=["observations probe fails", "observations probe unreadable", "Bronze probe fails"],
)
def test_a_probe_that_cannot_be_read_fails_the_heal(tmp_path: Path, staging: str, bronze: str, altered: int) -> None:
    run = _heal(tmp_path, staging=staging, bronze=bronze)

    assert run.returncode != 0, f"should fail: staging={staging!r} bronze={bronze!r}"
    assert len(run.altered()) == altered, "nothing past the unreadable probe is sent"
