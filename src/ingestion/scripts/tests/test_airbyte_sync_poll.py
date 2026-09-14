"""What the poll watchdog counts as progress.

The idle detector compares one tuple per poll — (status, bytes, records,
state messages) from the latest attempt — so what feeds that tuple decides
whether a stalled sync is caught or a slow one is killed.

Run: pytest src/ingestion/scripts/tests
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from airbyte_sync_poll import attempt_progress, job_status


@pytest.mark.parametrize(
    ("resp", "expected"),
    [
        ({}, None),
        ({"attempts": []}, None),
        (
            {
                "attempts": [
                    {
                        "attempt": {
                            "status": "running",
                            "totalStats": {"bytesEmitted": 10, "recordsEmitted": 2, "stateMessagesEmitted": 1},
                        }
                    }
                ]
            },
            ("running", 10, 2, 1),
        ),
        (
            {"attempts": [{"attempt": {"status": "running", "bytesSynced": 7, "recordsSynced": 3}}]},
            ("running", 7, 3, 0),
        ),
        ({"attempts": [{"attempt": {"status": "running"}}]}, ("running", 0, 0, 0)),
        (
            {
                "attempts": [
                    {"attempt": {"status": "failed", "totalStats": {"bytesEmitted": 1}}},
                    {"attempt": {"status": "running", "totalStats": {"bytesEmitted": 5}}},
                ]
            },
            ("running", 5, 0, 0),
        ),
    ],
)
def test_progress_reads_the_latest_attempts_counters(resp, expected) -> None:
    assert attempt_progress(resp) == expected, f"should read: {resp!r}"


@pytest.mark.parametrize(
    ("resp", "expected"),
    [
        ({"job": {"status": "running"}}, "running"),
        ({}, None),
        ({"job": {}}, None),
        ({"job": None}, None),
        ({"job": {"status": 7}}, None),
        ([], None),
        ("running", None),
    ],
)
def test_an_unusable_payload_yields_no_status_instead_of_raising(resp, expected) -> None:
    assert job_status(resp) == expected, f"should read: {resp!r}"
