"""Which dbt results become findings, and at what level.

Only data_quality-tagged test nodes emit; a pass is informational, anything
else warns — through the shared envelope, so the collector parses findings
like every other line.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

SCRIPTS = Path(__file__).resolve().parent.parent

RUN_RESULTS = {
    "metadata": {"invocation_id": "run-under-test"},
    "results": [
        {"unique_id": "test.ingestion.dq_pass", "status": "pass", "failures": 0, "execution_time": 0.5},
        {"unique_id": "test.ingestion.dq_fail", "status": "fail", "failures": 3, "execution_time": 1.0},
        {"unique_id": "test.ingestion.untagged", "status": "fail", "failures": 9},
        {"unique_id": "model.ingestion.some_model", "status": "success"},
    ],
}

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
            "tags": ["data_quality"],
            "config": {"severity": "warn", "meta": {}},
        },
        "test.ingestion.untagged": {"resource_type": "test", "name": "untagged", "tags": []},
        "model.ingestion.some_model": {"resource_type": "model", "name": "some_model"},
    }
}


def emit_findings(tmp_path: Path) -> list[dict]:
    target = tmp_path / "target"
    target.mkdir()
    (target / "run_results.json").write_text(json.dumps(RUN_RESULTS))
    (target / "manifest.json").write_text(json.dumps(MANIFEST))

    result = subprocess.run(
        [sys.executable, str(SCRIPTS / "data_quality_findings.py")],
        cwd=tmp_path,
        env={**os.environ, "INSIGHT_LOG_LEVEL": "info"},
        capture_output=True,
        text=True,
        check=True,
    )
    return [json.loads(line) for line in result.stderr.splitlines() if line]


def test_only_tagged_test_nodes_emit_and_status_picks_the_level(tmp_path) -> None:
    lines = emit_findings(tmp_path)

    by_check = {line["fields"]["check_id"]: line for line in lines}
    assert set(by_check) == {"dq_pass", "dq_fail"}

    assert by_check["dq_pass"]["level"] == "INFO"
    assert by_check["dq_fail"]["level"] == "WARN"
    assert by_check["dq_fail"]["fields"]["rows_violating"] == 3
    for line in lines:
        assert set(line) == {"timestamp", "level", "fields", "target"}
        assert line["fields"]["event"] == "data_quality_finding"
        assert line["fields"]["run_id"] == "run-under-test"
