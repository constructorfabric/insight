"""The sweep speaks the shared structured-log envelope (insight#3191).

Every stderr line from `python3 -m sweep` must be one JSON object of the agreed
shape — {"timestamp", "level", "fields", "target"} with target "sweep" — so the
collector pipeline parses the sweep like every other service, and the
install-wide INSIGHT_LOG_LEVEL knob must actually gate: at level=error a line
below error never reaches the pod log while the error itself does.

Drives the real module via subprocess with PYTHONPATH set the way sweep.sh sets
it — the python/ dir for the package, the scripts/ dir for insight_logging.

Run: pytest src/ingestion/reconcile-connectors/tests
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT.parent / "scripts"

AGREED_KEYS = {"timestamp", "level", "fields", "target"}


def run_sweep(stdin: str, level: str) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    env["PYTHONPATH"] = f"{ROOT / 'python'}{os.pathsep}{SCRIPTS}"
    env["INSIGHT_LOG_LEVEL"] = level
    env["RECONCILE_RUN_ID"] = "tick-under-test"
    return subprocess.run(["python3", "-m", "sweep"], input=stdin, env=env, capture_output=True, text=True, check=False)


def stderr_lines(result: subprocess.CompletedProcess) -> list[dict]:
    return [json.loads(line) for line in result.stderr.splitlines() if line]


def test_unreadable_work_logs_an_agreed_shape_error_line() -> None:
    """Unreadable work is an error, and even at level=error it must land
    on stderr as one agreed-shape JSON line targeted at the sweep."""
    result = run_sweep("not json", level="error")

    assert result.returncode == 1
    lines = stderr_lines(result)
    assert len(lines) == 1, result.stderr
    line = lines[0]
    assert set(line) == AGREED_KEYS, f"unexpected key set: {sorted(line)}"
    assert line["target"] == "sweep"
    assert line["level"] == "ERROR"
    assert "cannot read this tick's work" in line["fields"]["message"]
    assert line["fields"]["run_id"] == "tick-under-test"


def test_level_error_silences_lines_below_error() -> None:
    """The empty-connector refusal logs below error, so at level=error the
    pod log stays silent — the knob gates, it does not decorate."""
    work = json.dumps({"tick_id": "tick-1", "connectors": []})

    quiet = run_sweep(work, level="error")
    assert quiet.returncode == 1
    assert quiet.stderr == "", quiet.stderr

    chatty = run_sweep(work, level="info")
    lines = stderr_lines(chatty)
    assert len(lines) == 1, chatty.stderr
    assert set(lines[0]) == AGREED_KEYS
    assert lines[0]["target"] == "sweep"
