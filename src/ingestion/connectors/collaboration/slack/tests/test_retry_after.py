"""Retry-After parsing: delay-seconds, HTTP-date, junk, and the wait cap."""

from __future__ import annotations

from datetime import UTC, datetime, timedelta
from email.utils import format_datetime

import pytest
import requests
from source_slack.client import BACKOFF_BASE_SECS, MAX_WAIT_SECS, _wait


def _response(retry_after: str | None) -> requests.Response:
    response = requests.Response()
    if retry_after is not None:
        response.headers["Retry-After"] = retry_after
    return response


@pytest.mark.parametrize(("header", "expected"), [("7", 7.0), ("0", 0.0), ("0.5", 0.5), (" 3 ", 3.0)])
def test_delay_seconds(header: str, expected: float) -> None:
    assert _wait(_response(header), attempt=0) == expected


def test_http_date() -> None:
    when = datetime.now(UTC) + timedelta(seconds=30)

    assert 25 <= _wait(_response(format_datetime(when, usegmt=True)), attempt=0) <= 30


def test_http_date_in_the_past_waits_zero() -> None:
    when = datetime.now(UTC) - timedelta(minutes=5)

    assert _wait(_response(format_datetime(when, usegmt=True)), attempt=0) == 0.0


@pytest.mark.parametrize("header", ["²", "soon", "nan", "inf", "-inf", "", "Wed, 99 Foo 2026"])
def test_junk_falls_back_to_backoff(header: str) -> None:
    assert _wait(_response(header), attempt=2) == BACKOFF_BASE_SECS * 4


def test_negative_seconds_wait_zero() -> None:
    assert _wait(_response("-5"), attempt=0) == 0.0


def test_a_huge_ask_is_capped() -> None:
    assert _wait(_response("86400"), attempt=0) == MAX_WAIT_SECS


def test_backoff_is_capped_too() -> None:
    assert _wait(None, attempt=20) == MAX_WAIT_SECS
