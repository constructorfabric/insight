"""Zoom connector test config builder."""

from __future__ import annotations

import json

from connector_tests import HttpRequest, HttpResponse, ConfigBuilder

API_URL = "https://api.zoom.us/v2"
TOKEN_URL = "https://zoom.us/oauth/token"
METRICS_URL = f"{API_URL}/metrics/meetings"

# Frozen "now" every meetings/participants test uses. With this clock the
# DatetimeBasedCursor (start = now - P150D, step P30D, granularity P1D) slices
# the window into exactly these five from/to pairs — the final slice absorbs
# the remainder up to end_datetime instead of emitting a 1-day tail. The first
# slice starting at now-150d (inside Zoom's six-month Dashboard-API limit) is
# the job-529 regression pin.
FROZEN_NOW = "2026-07-01T00:00:00Z"
MEETING_SLICES = [
    ("2026-02-01", "2026-03-02"),
    ("2026-03-03", "2026-04-01"),
    ("2026-04-02", "2026-05-01"),
    ("2026-05-02", "2026-05-31"),
    ("2026-06-01", "2026-07-01"),
]

# The inline `_meetings` PARENT of participants is read through the CDK's
# synchronous path (SubstreamPartitionRouter.read_only_records), whose slice
# generator does NOT absorb the remainder into the last slice — it emits the
# plain step layout with a 1-day tail. Same window pin, different tail shape.
PARENT_MEETING_SLICES = MEETING_SLICES[:-1] + [
    ("2026-06-01", "2026-06-30"),
    ("2026-07-01", "2026-07-01"),
]


def metrics_params(from_date: str, to_date: str, page_token: str | None = None) -> dict:
    params = {"type": "past", "page_size": "100", "from": from_date, "to": to_date}
    if page_token:
        params["next_page_token"] = page_token
    return params


def mock_meeting_slices(http_mocker, non_empty: dict, slices=MEETING_SLICES) -> None:
    """Register every expected meetings slice; slices whose `from` date is not
    a key of `non_empty` serve an empty page. Exact from/to matchers mean an
    out-of-window request (the job-529 failure mode) matches nothing and fails
    the test."""
    for from_date, to_date in slices:
        http_mocker.get(
            HttpRequest(METRICS_URL, query_params=metrics_params(from_date, to_date)),
            non_empty.get(
                from_date,
                HttpResponse(body=json.dumps({"meetings": []}), status_code=200),
            ),
        )


class ZoomConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "zoom_account_id": "test-account",
                "zoom_client_id": "test-client",
                "zoom_client_secret": "test-secret",
                # Explicit so exact request matchers can pin page_size=100 on
                # every stream (the spec default only applies when Airbyte
                # injects it into the config).
                "page_size": 100,
            }
        )


def mock_token(http_mocker) -> None:
    """Register the Server-to-Server OAuth token exchange every stream's
    SessionTokenAuthenticator performs before its first API call. The matcher
    compares the form body byte-exact, so this also pins the
    account_credentials grant the manifest declares."""
    http_mocker.post(
        HttpRequest(
            TOKEN_URL,
            body="grant_type=account_credentials&account_id=test-account",
        ),
        HttpResponse(
            body=json.dumps(
                {"access_token": "test-bearer", "token_type": "bearer", "expires_in": 3600}
            ),
            status_code=200,
        ),
    )
