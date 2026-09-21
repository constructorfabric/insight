"""Job status and optional progress telemetry from Airbyte responses."""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from airbyte_sync_poll import FAILURE_MESSAGE_MAX_CHARS, SyncFailure, attempt_failures, attempt_progress, job_status


def _resp_with_failures(failures: object) -> dict:
    return {"attempts": [{"attempt": {"status": "failed", "failureSummary": {"failures": failures}}}]}


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
        ({"job": {"status": "unexpected"}}, None),
        ([], None),
        ("running", None),
    ],
)
def test_an_unusable_payload_yields_no_status_instead_of_raising(resp, expected) -> None:
    assert job_status(resp) == expected, f"should read: {resp!r}"


def test_every_reason_airbyte_recorded_is_read_from_the_latest_attempt() -> None:
    superseded = {"attempt": {"status": "failed", "failureSummary": {"failures": [{"failureType": "stale"}]}}}
    latest = _resp_with_failures(
        [
            {
                "failureType": "config_error",
                "failureOrigin": "source",
                "externalMessage": "HTTP Status Code: 401.",
                "internalMessage": "credentials rejected",
            },
            {"failureType": "system_error", "failureOrigin": "source"},
        ]
    )

    resp = {"attempts": [superseded, *latest["attempts"]]}

    assert attempt_failures(resp) == [
        SyncFailure("config_error", "source", "HTTP Status Code: 401.", "credentials rejected"),
        SyncFailure("system_error", "source", "", ""),
    ]


@pytest.mark.parametrize(
    "resp",
    [
        {},
        [],
        "failed",
        {"attempts": []},
        {"attempts": [{"attempt": {"status": "failed"}}]},
        {"attempts": [{"attempt": {"status": "failed", "failureSummary": {}}}]},
        {"attempts": [{"attempt": None}]},
        {"attempts": ["failed"]},
        _resp_with_failures(None),
        _resp_with_failures("boom"),
        _resp_with_failures(["boom"]),
    ],
)
def test_a_payload_carrying_no_usable_reason_yields_no_reasons_instead_of_raising(resp) -> None:
    assert attempt_failures(resp) == [], f"should read: {resp!r}"


def test_a_reason_missing_its_classification_reads_as_unknown_rather_than_empty() -> None:
    assert attempt_failures(_resp_with_failures([{}])) == [SyncFailure("unknown", "unknown", "", "")]


@pytest.mark.parametrize("value", [["a", "b"], {"k": "v"}, 7, 1.5, True, None])
def test_a_field_that_is_not_text_reads_as_absent_rather_than_stringified(value) -> None:
    entry = {"failureType": value, "failureOrigin": value, "externalMessage": value, "internalMessage": value}

    assert attempt_failures(_resp_with_failures([entry])) == [SyncFailure("unknown", "unknown", "", "")], (
        f"should reject: {value!r}"
    )


def test_an_oversized_reason_is_truncated_so_one_failure_cannot_flood_the_log() -> None:
    long_message = "x" * (FAILURE_MESSAGE_MAX_CHARS * 3)

    (failure,) = attempt_failures(
        _resp_with_failures([{"externalMessage": long_message, "internalMessage": long_message}])
    )

    assert failure.external_message == "x" * FAILURE_MESSAGE_MAX_CHARS
    assert failure.internal_message == "x" * FAILURE_MESSAGE_MAX_CHARS
