"""A retype heal moves a column to the type the current model creates, and
only there: a narrowing first gives the NULL rows the model's fallback, a
column already at the type is left alone, and a probe that did not answer
stops the hook rather than reading as "nothing to do".

The helpers are lifted out of the deploy script and run against a stubbed
transport, since sourcing the script would run the deploy.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parents[1]
EXEC_LIB = SCRIPTS / "lib" / "ch-exec.sh"
HEALS = SCRIPTS / "apply-ch-migrations.sh"

REACHED_THE_END = "HOOK CONTINUED"
PROBE_FAILS = "FAILS"


def function_body(name: str) -> str:
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", HEALS.read_text(), re.MULTILINE | re.DOTALL)
    assert match, f"{HEALS.name} no longer defines {name}"
    return match.group(0)


def run(
    call: str, column_type: str, null_rows: str, tmp_path: Path
) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    """`column_type` answers the system.columns probe (PROBE_FAILS fails it),
    `null_rows` answers the NULL count; every other statement is logged and
    answered with nothing."""
    log = tmp_path / "statements.log"
    log.touch()
    script = f"""
set -euo pipefail
export CLICKHOUSE_URL=http://127.0.0.1:1 CLICKHOUSE_USER=u CLICKHOUSE_PASSWORD=p
source "{EXEC_LIB}"
_ch_http_query() {{
  local sql
  sql="$(cat)"
  printf '%s\\n;;\\n' "$sql" >> "$LOG"
  case "$sql" in
    *"FROM system.columns"*)
      if [[ "$COLUMN_TYPE" == "{PROBE_FAILS}" ]]; then
        printf 'Code: 999. DB::Exception: Service unavailable\\n'; return 22
      fi
      printf '%s\\n' "$COLUMN_TYPE" ;;
    *"count() FROM"*) printf '%s\\n' "$NULL_ROWS" ;;
    *) ;;
  esac
}}
{function_body("retype_column")}
{function_body("retype_column_not_null")}
{call}
echo '{REACHED_THE_END}'
"""
    result = subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={"PATH": "/usr/bin:/bin", "LOG": str(log), "COLUMN_TYPE": column_type, "NULL_ROWS": null_rows},
        check=False,
    )
    statements = [s.strip() for s in log.read_text().split("\n;;\n") if s.strip()]
    return result, statements


def alters(statements: list[str]) -> list[str]:
    return [s for s in statements if s.startswith("ALTER TABLE")]


NARROW = "retype_column_not_null staging jira__worklog_snapshot is_deleted UInt8 0"
WIDEN = "retype_column staging cursor__ai_dev_usage lines_removed 'Nullable(UInt32)'"


def test_a_narrowing_gives_the_null_rows_the_fallback_before_the_type_changes(tmp_path: Path) -> None:
    result, statements = run(NARROW, "Nullable(UInt8)", "3", tmp_path)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout
    assert alters(statements) == [
        "ALTER TABLE staging.jira__worklog_snapshot UPDATE is_deleted = 0 WHERE is_deleted IS NULL SETTINGS mutations_sync = 2",
        "ALTER TABLE staging.jira__worklog_snapshot MODIFY COLUMN is_deleted UInt8",
    ]


def test_a_narrowing_with_no_null_row_changes_the_type_alone(tmp_path: Path) -> None:
    result, statements = run(NARROW, "Nullable(UInt8)", "0", tmp_path)

    assert result.returncode == 0, result.stderr
    assert alters(statements) == ["ALTER TABLE staging.jira__worklog_snapshot MODIFY COLUMN is_deleted UInt8"]


@pytest.mark.parametrize("column_type", ["UInt8", "String", ""])
def test_a_narrowing_touches_only_the_nullable_form_of_its_type(column_type: str, tmp_path: Path) -> None:
    result, statements = run(NARROW, column_type, "3", tmp_path)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout
    assert not alters(statements), f"the heal altered a column typed {column_type!r}"


def test_a_null_count_the_server_did_not_answer_stops_the_hook(tmp_path: Path) -> None:
    result, statements = run(NARROW, "Nullable(UInt8)", "<html>502 Bad Gateway</html>", tmp_path)

    assert result.returncode != 0, result
    assert REACHED_THE_END not in result.stdout, "the hook carried on over a count it could not read"
    assert not alters(statements)


def test_a_widening_changes_a_column_at_another_type(tmp_path: Path) -> None:
    result, statements = run(WIDEN, "UInt32", "0", tmp_path)

    assert result.returncode == 0, result.stderr
    assert alters(statements) == [
        "ALTER TABLE staging.cursor__ai_dev_usage MODIFY COLUMN lines_removed Nullable(UInt32)"
    ]


@pytest.mark.parametrize("column_type", ["Nullable(UInt32)", ""])
def test_a_widening_leaves_a_converged_or_absent_column_alone(column_type: str, tmp_path: Path) -> None:
    result, statements = run(WIDEN, column_type, "0", tmp_path)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout
    assert not alters(statements)


@pytest.mark.parametrize("call", [NARROW, WIDEN])
def test_a_type_probe_the_server_did_not_answer_stops_the_hook(call: str, tmp_path: Path) -> None:
    result, statements = run(call, PROBE_FAILS, "0", tmp_path)

    assert result.returncode != 0, result
    assert REACHED_THE_END not in result.stdout, "the hook carried on over a type it could not read"
    assert not alters(statements), "the heal altered a column whose type it could not read"
