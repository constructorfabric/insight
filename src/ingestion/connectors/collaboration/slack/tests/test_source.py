"""Spec and connection check."""

from __future__ import annotations

import logging

import pytest
import requests_mock as rm
from conftest import CONFIG, day_url, member_file, refusal
from source_slack.source import SourceSlack

_LOG = logging.getLogger("test")


def test_spec_requires_the_insight_identity_and_the_token() -> None:
    spec = SourceSlack().spec(_LOG).connectionSpecification

    assert set(spec["required"]) == {"insight_tenant_id", "insight_source_id", "slack_bot_token", "slack_start_date"}
    assert spec["properties"]["slack_bot_token"]["airbyte_secret"] is True


def test_check_passes_when_slack_serves_the_latest_day() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        assert SourceSlack().check_connection(_LOG, CONFIG) == (True, None)


def test_check_passes_when_the_latest_day_is_not_published_yet() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-02"), **refusal("file_not_yet_available"))

        assert SourceSlack().check_connection(_LOG, CONFIG) == (True, None)


def test_check_names_slacks_refusal() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-02"), **refusal("not_allowed_token_type"))

        ok, message = SourceSlack().check_connection(_LOG, CONFIG)

    assert not ok
    assert "not_allowed_token_type" in message


def test_check_reports_an_unreachable_api() -> None:
    import requests

    with rm.Mocker() as http:
        http.get(day_url("2026-10-02"), exc=requests.ConnectionError("no route"))

        ok, message = SourceSlack().check_connection(_LOG, CONFIG)

    assert not ok
    assert "no route" in message


@pytest.mark.parametrize("field", ["insight_tenant_id", "insight_source_id", "slack_bot_token", "slack_start_date"])
def test_check_rejects_an_empty_required_field(field: str) -> None:
    ok, message = SourceSlack().check_connection(_LOG, {**CONFIG, field: " "})

    assert not ok
    assert field in message


def test_check_rejects_a_malformed_start_date() -> None:
    ok, message = SourceSlack().check_connection(_LOG, {**CONFIG, "slack_start_date": "01/10/2026"})

    assert not ok
    assert "YYYY-MM-DD" in message
