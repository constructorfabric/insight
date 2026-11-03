"""A gold identity backup is dropped only once its live relation is a View.

`<table>__pre_view_backup` holds the rows of a relation that stood as a table,
kept so an operator can rename it back. Dropping it is safe only when the view
already stands in its place; a failed engine lookup must stop the deploy rather
than read as either answer.

The function is lifted out of the deploy script and run against stubbed
helpers, since sourcing the script would run the deploy.
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parents[1]
HEALS = SCRIPTS / "apply-ch-migrations.sh"

CLEANUP = "drop_superseded_gold_backup"
DROP = "DROP TABLE IF EXISTS insight.metric_entity_cohorts_current__pre_view_backup"
REACHED_THE_END = "HOOK CONTINUED"


def function_body(name: str) -> str:
    match = re.search(rf"^{name}\(\) \{{\n.*?^\}}\n", HEALS.read_text(), re.MULTILINE | re.DOTALL)
    assert match, f"{HEALS.name} no longer defines {name}"
    return match.group(0)


def run(*, backup: bool, engine: str) -> subprocess.CompletedProcess[str]:
    """`engine` is what the lookup answers; `<fail>` makes the lookup fail."""
    script = f"""
set -euo pipefail
ch_table_exists() {{ [[ "$2" == *__pre_view_backup && "$BACKUP" == 1 ]]; }}
ch_table_engine() {{ [[ "$ENGINE" == "<fail>" ]] && return 1; printf '%s' "$ENGINE"; }}
run_ch() {{ cat; }}
{function_body(CLEANUP)}
{CLEANUP} insight metric_entity_cohorts_current
echo '{REACHED_THE_END}'
"""
    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        env={"PATH": "/usr/bin:/bin", "BACKUP": "1" if backup else "0", "ENGINE": engine},
        check=False,
    )


def test_a_backup_superseded_by_the_view_is_dropped() -> None:
    result = run(backup=True, engine="View")

    assert result.returncode == 0, result.stderr
    assert DROP in result.stdout, result.stdout
    assert REACHED_THE_END in result.stdout


def test_a_backup_whose_live_relation_is_a_table_is_kept() -> None:
    result = run(backup=True, engine="MergeTree")

    assert result.returncode == 0, result.stderr
    assert "DROP" not in result.stdout, result.stdout
    assert REACHED_THE_END in result.stdout


def test_nothing_happens_without_a_backup() -> None:
    result = run(backup=False, engine="View")

    assert result.returncode == 0, result.stderr
    assert "DROP" not in result.stdout, result.stdout
    assert REACHED_THE_END in result.stdout


def test_a_lookup_that_did_not_run_stops_the_hook_without_a_drop() -> None:
    result = run(backup=True, engine="<fail>")

    assert result.returncode != 0, result
    assert "DROP" not in result.stdout, result.stdout
    assert REACHED_THE_END not in result.stdout, "a failed engine lookup let the hook continue"
