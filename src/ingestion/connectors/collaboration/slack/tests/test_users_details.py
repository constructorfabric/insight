"""users_details read through the CDK entrypoint against a mocked Slack.

`requests_mock` raises NoMockAddress on any unregistered URL, so a read that
asks for a day it should not ask for fails the test.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
import requests
import requests_mock as rm
from airbyte_cdk.models import FailureType, SyncMode
from airbyte_cdk.test.entrypoint_wrapper import discover
from conftest import CONFIG, catalog, day_url, last_state, member_file, refusal, run, stream_state
from jsonschema import Draft7Validator
from source_slack.source import SourceSlack

_SCHEMA = json.loads((Path(__file__).parents[1] / "source_slack" / "schemas" / "users_details.json").read_text())


def test_reads_each_day_up_to_the_publication_delay_and_stamps_identity() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **member_file("2026-10-01"))
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run()

    records = [r.record.data for r in output.records]
    assert [(r["date"], r["user_id"]) for r in records] == [
        ("2026-10-01", "U0000000001"),
        ("2026-10-01", "U0000000002"),
        ("2026-10-02", "U0000000001"),
        ("2026-10-02", "U0000000002"),
    ]
    assert records[0]["unique_key"] == "test-tenant-slack-test-U0000000001-2026-10-01"
    assert {r["tenant_id"] for r in records} == {"test-tenant"}
    assert {r["source_id"] for r in records} == {"slack-test"}
    for record in records:
        assert not list(Draft7Validator(_SCHEMA).iter_errors(record))
    assert last_state(output) == {"date": "2026-10-02"}
    assert http.request_history[0].headers["Authorization"] == "Bearer xoxp-test-token"


def test_checkpoints_after_every_day() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **member_file("2026-10-01"))
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run()

    days = [m.state.stream.stream_state.__dict__.get("date") for m in output.state_messages]
    assert "2026-10-01" in days and days[-1] == "2026-10-02"


def test_resumes_from_the_declarative_cursor_day_inclusive() -> None:
    config = {**CONFIG, "slack_start_date": "2026-09-01"}
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **member_file("2026-10-01"))
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run(stream_state({"date": "2026-10-01"}), config=config)

    assert [r.url for r in http.request_history] == [day_url("2026-10-01"), day_url("2026-10-02")]
    assert last_state(output) == {"date": "2026-10-02"}


def test_a_cursor_at_the_publication_edge_reads_only_that_day() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run(stream_state({"date": "2026-10-02"}))

    assert {r.record.data["date"] for r in output.records} == {"2026-10-02"}
    assert not output.errors


def test_a_cursor_past_the_publication_edge_asks_for_nothing() -> None:
    with rm.Mocker():
        output = run(stream_state({"date": "2026-10-03"}))

    assert not output.records
    assert not output.errors


@pytest.mark.parametrize("code", ["missing_scope", "invalid_auth", "token_revoked", "feature_not_available"])
def test_a_refused_token_fails_the_sync_as_a_config_error(code: str) -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **refusal(code))

        output = run(expecting_exception=True)

    assert not output.records
    errors = [e.trace.error for e in output.errors]
    assert FailureType.config_error in [e.failure_type for e in errors]
    assert any(code in (e.message or "") and "2026-10-01" in (e.message or "") for e in errors)


def test_a_missing_day_fails_the_sync_and_keeps_the_days_before_it() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **member_file("2026-10-01"))
        http.get(day_url("2026-10-02"), **refusal("file_not_found"))

        output = run(expecting_exception=True)

    assert {r.record.data["date"] for r in output.records} == {"2026-10-01"}
    assert last_state(output) == {"date": "2026-10-01"}
    errors = [e.trace.error for e in output.errors]
    assert FailureType.system_error in [e.failure_type for e in errors]
    assert any("file_not_found" in (e.message or "") for e in errors)


def test_a_day_not_yet_published_ends_the_sync_cleanly_at_the_last_published_day() -> None:
    config = {**CONFIG, "slack_start_date": "2026-09-30"}
    with rm.Mocker() as http:
        http.get(day_url("2026-09-30"), **member_file("2026-09-30"))
        http.get(day_url("2026-10-01"), **refusal("file_not_yet_available"))

        output = run(config=config)

    assert not output.errors
    assert {r.record.data["date"] for r in output.records} == {"2026-09-30"}
    assert last_state(output) == {"date": "2026-09-30"}
    # 2026-10-02 is never asked for: a later day cannot be ready either.
    assert [r.url for r in http.request_history] == [day_url("2026-09-30"), day_url("2026-10-01")]


def test_a_day_still_unpublished_past_the_grace_period_fails() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-09-20"), **refusal("file_not_yet_available"))

        output = run(
            stream_state({"date": "2026-09-20"}),
            expecting_exception=True,
            config={**CONFIG, "slack_start_date": "2026-09-01"},
        )

    assert not output.records
    assert any("file_not_yet_available" in (e.trace.error.message or "") for e in output.errors)


def test_the_configured_start_date_overrides_an_older_cursor() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **member_file("2026-10-01"))
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run(stream_state({"date": "2026-09-19"}))

    assert {r.record.data["date"] for r in output.records} == {"2026-10-01", "2026-10-02"}


def test_retries_a_rate_limit_after_the_wait_slack_asks_for(sleeps: list[float]) -> None:
    with rm.Mocker() as http:
        http.get(
            day_url("2026-10-01"), [{"status_code": 429, "headers": {"Retry-After": "7"}}, member_file("2026-10-01")]
        )
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run()

    assert len(output.records) == 4
    assert sleeps == [7.0]


def test_retries_a_temporary_refusal_in_the_body(sleeps: list[float]) -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), [refusal("ratelimited"), member_file("2026-10-01")])
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run()

    assert len(output.records) == 4
    assert len(sleeps) == 1


def test_a_temporary_refusal_that_persists_fails_the_sync() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **refusal("service_unavailable"))

        output = run(expecting_exception=True)

    assert http.call_count == 6
    assert any("service_unavailable" in (e.trace.error.message or "") for e in output.errors)


def test_retries_a_network_error_then_reads_the_day(sleeps: list[float]) -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), [{"exc": requests.ConnectionError("reset")}, member_file("2026-10-01")])
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = run()

    assert len(output.records) == 4
    assert len(sleeps) == 1


def test_a_network_error_that_persists_fails_as_transient() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), exc=requests.ConnectTimeout("timed out"))

        output = run(expecting_exception=True)

    assert http.call_count == 6
    assert FailureType.transient_error in [e.trace.error.failure_type for e in output.errors]


def test_a_server_error_that_persists_fails_the_sync() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), status_code=503)

        output = run(expecting_exception=True)

    assert http.call_count == 6
    assert any("HTTP 503" in (e.trace.error.message or "") for e in output.errors)


def test_a_body_that_is_neither_gzip_nor_json_fails_the_sync() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), content=b"<html>maintenance</html>", status_code=200)

        output = run(expecting_exception=True)

    messages = [e.trace.error.message or "" for e in output.errors]
    assert any("not JSON Lines" in m for m in messages)
    assert not any("maintenance" in m for m in messages), "the body must not be echoed"


def test_a_json_body_that_is_not_a_refusal_fails_the_sync() -> None:
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), json={"ok": True}, status_code=200)

        output = run(expecting_exception=True)

    assert any("without a member file" in (e.trace.error.message or "") for e in output.errors)


def test_a_member_file_that_does_not_parse_never_echoes_member_data() -> None:
    body = b'{"user_id": "U1", "email_address": "secret@example.com"}\n{broken'
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), content=body, status_code=200)

        output = run(expecting_exception=True)

    assert output.errors
    assert not any("secret@example.com" in (e.trace.error.message or "") for e in output.errors)


def test_a_row_without_user_id_fails_the_sync() -> None:
    import gzip

    body = gzip.compress(json.dumps({"date": "2026-10-01", "email_address": "x@example.com"}).encode())
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), content=body, status_code=200)

        output = run(expecting_exception=True)

    assert any("without user_id" in (e.trace.error.message or "") for e in output.errors)


def test_discover_offers_incremental_on_a_source_defined_date_cursor() -> None:
    output = discover(SourceSlack(), CONFIG)

    (stream,) = output.catalog.catalog.streams
    assert stream.name == "users_details"
    assert SyncMode.incremental in stream.supported_sync_modes
    assert stream.source_defined_cursor
    assert stream.default_cursor_field == ["date"]
    assert stream.source_defined_primary_key == [["unique_key"]]


def test_full_refresh_reads_the_whole_window() -> None:
    from airbyte_cdk.test.entrypoint_wrapper import read

    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **member_file("2026-10-01"))
        http.get(day_url("2026-10-02"), **member_file("2026-10-02"))

        output = read(SourceSlack(), CONFIG, catalog(SyncMode.full_refresh))

    assert len(output.records) == 4


def test_a_body_streamed_past_the_compressed_limit_fails_the_sync(monkeypatch: pytest.MonkeyPatch) -> None:
    from source_slack import client as client_module

    monkeypatch.setattr(client_module, "MAX_COMPRESSED_BYTES", 64)
    with rm.Mocker() as http:
        http.get(day_url("2026-10-01"), **member_file("2026-10-01"))

        output = run(expecting_exception=True)

    assert any("exceeds 64 bytes" in (e.trace.error.message or "") for e in output.errors)


@pytest.mark.parametrize("declared", ["garbage", "-1", "12abc"])
def test_a_malformed_content_length_is_left_to_the_streaming_cap(
    declared: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    from source_slack import client as client_module

    monkeypatch.setattr(client_module, "MAX_COMPRESSED_BYTES", 64)
    file = member_file("2026-10-01")
    with rm.Mocker() as http:
        http.get(
            day_url("2026-10-01"), content=file["content"], headers={**file["headers"], "Content-Length": declared}
        )

        output = run(expecting_exception=True)

    assert any("body exceeds 64 bytes" in (e.trace.error.message or "") for e in output.errors)
