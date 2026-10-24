"""A heal whose table probe did not answer stops the deploy; one answered
"absent" is skipped.

Every heal in apply-ch-migrations.sh is guarded as `ch_table_exists ... ||
return 0`, and `||` swallows whatever the probe returns. Behind a balancer a
503 from a restarting replica would therefore read as "no such table": the
heal is skipped, the hook reports success, and the next dbt run meets the
unhealed shape. The probe has to tell the two apart itself.

A heal is lifted out of the deploy script and run against a stubbed
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

HEAL = "heal_zoom_participant_device_columns"
HEAL_STATEMENT = "ALTER TABLE bronze_zoom.participants"
REACHED_THE_END = "HOOK CONTINUED"

#: curl's exit status under --fail-with-body for an HTTP error; the body is
#: what ClickHouse puts on a 503.
SERVICE_UNAVAILABLE = "printf 'Code: 999. DB::Exception: Service unavailable\\n'; return 22"


def function_body(name: str) -> str:
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", HEALS.read_text(), re.MULTILINE | re.DOTALL)
    assert match, f"{HEALS.name} no longer defines {name}"
    return match.group(0)


def run(probe_answer: str, tmp_path: Path) -> tuple[subprocess.CompletedProcess[str], list[str]]:
    """`probe_answer` is the shell that answers the system.tables probe; every
    other statement is logged and answered with nothing."""
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
    *"FROM system.tables"*) {probe_answer} ;;
    *) ;;
  esac
}}
{function_body(HEAL)}
{HEAL}
echo '{REACHED_THE_END}'
"""
    result = subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={"PATH": "/usr/bin:/bin", "LOG": str(log)},
        check=False,
    )
    statements = [s.strip() for s in log.read_text().split("\n;;\n") if s.strip()]
    return result, statements


def test_a_probe_the_server_did_not_answer_stops_the_hook(tmp_path: Path) -> None:
    result, statements = run(SERVICE_UNAVAILABLE, tmp_path)

    assert result.returncode != 0, result
    assert REACHED_THE_END not in result.stdout, "the hook carried on over a heal it could not decide"
    assert not [s for s in statements if HEAL_STATEMENT in s], "the heal ran on an undecided table"
    assert "system.tables" in result.stderr and "bronze_zoom.participants" in result.stderr, result.stderr


@pytest.mark.parametrize("answer", ["printf ''", "printf '<html>502 Bad Gateway</html>\\n'"])
def test_an_answer_that_is_not_a_count_is_not_read_as_absent(answer: str, tmp_path: Path) -> None:
    result, statements = run(answer, tmp_path)

    assert result.returncode != 0, result
    assert REACHED_THE_END not in result.stdout, f"the hook read {answer!r} as an absent table"
    assert not [s for s in statements if HEAL_STATEMENT in s]


def test_a_table_the_server_answered_absent_skips_the_heal(tmp_path: Path) -> None:
    result, statements = run("printf '0\\n'", tmp_path)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout, result
    assert not [s for s in statements if HEAL_STATEMENT in s], "the heal ran on a table that does not exist"


def test_a_table_the_server_answered_present_is_healed(tmp_path: Path) -> None:
    result, statements = run("printf '1\\n'", tmp_path)

    assert result.returncode == 0, result.stderr
    assert REACHED_THE_END in result.stdout, result
    assert [s for s in statements if HEAL_STATEMENT in s], "the heal was skipped on a table that exists"


def test_no_heal_guard_runs_the_probe_where_its_exit_cannot_reach_the_hook() -> None:
    """The probe stops the hook by exiting; a subshell, a pipeline or a command
    substitution would swallow that exit and hand the guard a status again."""
    guards = [
        f"{source.name}:{number}: {line.strip()}"
        for source in (HEALS, SCRIPTS / "lib" / "heal-jira-catalogue-first-seen.sh")
        for number, line in enumerate(source.read_text().splitlines(), start=1)
        if "ch_table_exists" in line and not line.strip().startswith("#")
    ]

    assert guards, "no heal is guarded by ch_table_exists any more"
    offenders = [
        g for g in guards if re.search(r"\$\(\s*ch_table_exists|\(\s*ch_table_exists|ch_table_exists[^|]*\|[^|]", g)
    ]
    assert not offenders, f"the probe's exit would not reach the hook here: {offenders}"
