"""Which dbt results become findings, at what level, and how the run reports itself.

Only quality-tagged test nodes emit; a pass is informational, anything else
warns — through the shared envelope, so the collector parses findings like
every other line. One run line follows, failed when dbt or a check broke.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPTS = Path(__file__).resolve().parent.parent

PASS = {"unique_id": "test.ingestion.dq_pass", "status": "pass", "failures": 0, "execution_time": 0.5}
FAIL = {"unique_id": "test.ingestion.dq_fail", "status": "warn", "failures": 3, "execution_time": 1.0}
ERRORED = {"unique_id": "test.ingestion.dq_fail", "status": "error", "failures": None}
UNTAGGED = {"unique_id": "test.ingestion.untagged", "status": "fail", "failures": 9}
MODEL = {"unique_id": "model.ingestion.some_model", "status": "success"}

MANIFEST = {
    "nodes": {
        "test.ingestion.dq_pass": {
            "resource_type": "test",
            "name": "dq_pass",
            "tags": ["data_quality"],
            "config": {"severity": "warn", "meta": {"title": "Pass check", "domain": "git"}},
        },
        "test.ingestion.dq_fail": {
            "resource_type": "test",
            "name": "dq_fail",
            "tags": ["connector_quality"],
            "config": {"severity": "warn", "meta": {"tier": "error"}},
        },
        "test.ingestion.untagged": {"resource_type": "test", "name": "untagged", "tags": []},
        "model.ingestion.some_model": {"resource_type": "model", "name": "some_model"},
    }
}

POST_SYNC = ["--scope", "post_sync", "--connector", "example"]


def emit(tmp_path: Path, args: list[str], results: list[dict] | None) -> list[dict]:
    if results is not None:
        target = tmp_path / "target"
        target.mkdir()
        run_results = {"metadata": {"invocation_id": "run-under-test"}, "results": results}
        (target / "run_results.json").write_text(json.dumps(run_results))
        (target / "manifest.json").write_text(json.dumps(MANIFEST))

    completed = subprocess.run(
        [sys.executable, str(SCRIPTS / "data_quality_findings.py"), *args],
        cwd=tmp_path,
        env={**os.environ, "INSIGHT_LOG_LEVEL": "warn"},
        capture_output=True,
        text=True,
        check=True,
    )
    return [json.loads(line) for line in completed.stderr.splitlines() if line]


def by_event(lines: list[dict], event: str) -> list[dict]:
    return [line for line in lines if line["fields"]["event"] == event]


def test_only_tagged_test_nodes_emit_and_status_picks_the_level(tmp_path) -> None:
    lines = emit(tmp_path, [*POST_SYNC, "--dbt-exit-code", "0"], [PASS, FAIL, UNTAGGED, MODEL])

    by_check = {line["fields"]["check_id"]: line for line in by_event(lines, "data_quality_finding")}
    assert set(by_check) == {"dq_pass", "dq_fail"}

    assert by_check["dq_pass"]["level"] == "INFO"
    assert by_check["dq_fail"]["level"] == "WARN"
    assert by_check["dq_fail"]["fields"]["rows_violating"] == 3
    assert by_check["dq_fail"]["fields"]["tier"] == "error"
    for line in lines:
        assert set(line) == {"timestamp", "level", "fields", "target"}
        assert line["fields"]["run_id"] == "run-under-test"
        assert line["fields"]["scope"] == "post_sync"
        assert line["fields"]["connector"] == "example"


def test_passes_survive_a_quieter_install_level_so_alerts_can_resolve(tmp_path) -> None:
    lines = emit(tmp_path, [*POST_SYNC, "--dbt-exit-code", "0"], [PASS])

    assert [line["fields"]["check_id"] for line in by_event(lines, "data_quality_finding")] == ["dq_pass"]
    assert [line["fields"]["failed"] for line in by_event(lines, "data_quality_run")] == [0]


@pytest.mark.parametrize(
    ("exit_code", "results", "failed"),
    [(0, [PASS, FAIL], 0), (1, [PASS, FAIL], 1), (1, [PASS, ERRORED], 1), (2, None, 1)],
    ids=["findings-only", "dbt-nonzero", "check-errored", "no-artifacts"],
)
def test_run_line_fails_only_on_an_operational_problem(tmp_path, exit_code, results, failed) -> None:
    lines = emit(tmp_path, ["--scope", "scheduled", "--dbt-exit-code", str(exit_code)], results)

    runs = by_event(lines, "data_quality_run")
    assert len(runs) == 1, f"expected one run line: {lines!r}"
    assert runs[0]["fields"]["failed"] == failed, f"wrong outcome for exit {exit_code}: {runs[0]!r}"
    assert runs[0]["fields"]["scope"] == "scheduled"
    assert runs[0]["level"] == ("ERROR" if failed else "INFO")
