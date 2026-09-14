"""What the trigger step accepts as a successful sync answer.

stdout is the step's Argo outputs.result, so a payload without a usable job id
must fail structuredly — never print garbage for poll-job to consume, and
never escape as a traceback.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from airbyte_sync_trigger import triggered_job_id


@pytest.mark.parametrize(
    ("resp", "expected"),
    [
        ({"job": {"id": 42}}, 42),
        ({"job": {"id": "42"}}, "42"),
        ({}, None),
        ({"job": {}}, None),
        ({"job": None}, None),
        ({"job": {"id": None}}, None),
        ([], None),
        ("42", None),
    ],
)
def test_only_a_usable_job_id_reaches_stdout(resp, expected) -> None:
    assert triggered_job_id(resp) == expected, f"should read: {resp!r}"
