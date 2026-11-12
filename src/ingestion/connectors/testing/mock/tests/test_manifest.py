from __future__ import annotations

import json
import logging

import pytest
from airbyte_cdk.models import Status, SyncMode
from connector_tests import (
    ConfigBuilder,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    get_source,
    load_fixture,
    read_stream,
)
from freezegun import freeze_time

CONNECTOR = "testing/mock"
BASE_URL = "http://mock-source.example.com/instances/mock-a"
STREAMS = ("mock_events", "mock_updates")


def config() -> dict[str, object]:
    return ConfigBuilder().with_field("base_url", BASE_URL).with_field("api_token", "synthetic-token").build()


def record(stream: str, index: int) -> dict[str, object]:
    return load_fixture(__file__, "record.json", id=f"{stream}-{index}", last_updated_at=f"2026-01-01T00:00:0{index}Z")


def page(rows: list[dict[str, object]], token: str | None = None) -> HttpResponse:
    body = load_fixture(__file__, "page.json", records=rows, next_page_token=token)
    return HttpResponse(body=json.dumps(body), status_code=200)


def query(stream: str) -> dict[str, str]:
    return {"cursor": "1970-01-01T00:00:00Z"} if stream == "mock_updates" else {}


@pytest.mark.parametrize("stream", STREAMS)
@freeze_time("2026-02-01")
def test_pagination_preserves_identity_and_schema(http_mocker: HttpMocker, stream: str) -> None:
    headers = {"Authorization": "Bearer synthetic-token"}
    url = f"{BASE_URL}/streams/{stream}"
    http_mocker.get(
        HttpRequest(url, query_params=query(stream), headers=headers),
        page([record(stream, 0), record(stream, 1)], "synthetic-read:2"),
    )
    http_mocker.get(
        HttpRequest(url, query_params={**query(stream), "page_token": "synthetic-read:2"}, headers=headers),
        page([record(stream, 2)]),
    )
    output = read_stream(CONNECTOR, stream, config())
    assert not output.errors
    assert [row.record.data["id"] for row in output.records] == [f"{stream}-{i}" for i in range(3)]
    for row in output.records:
        assert row.record.data["tenant_id"] == "test-tenant"
        assert row.record.data["source_id"] == "test-source"
        assert row.record.data["unique_key"] == f"test-tenant-test-source-{row.record.data['id']}"
    assert_records_conform(output.records, CONNECTOR, stream)


@pytest.mark.parametrize("stream", STREAMS)
@freeze_time("2026-02-01")
def test_empty_stream_terminates_successfully(http_mocker: HttpMocker, stream: str) -> None:
    http_mocker.get(HttpRequest(f"{BASE_URL}/streams/{stream}", query_params=query(stream)), page([]))
    output = read_stream(CONNECTOR, stream, config())
    assert not output.records
    assert not output.errors


@pytest.mark.parametrize("status", [429, 503])
def test_transient_error_retries_without_losing_records(http_mocker: HttpMocker, status: int) -> None:
    http_mocker.get(
        HttpRequest(f"{BASE_URL}/streams/mock_events", query_params={}),
        [
            HttpResponse(
                body=json.dumps(load_fixture(__file__, "error.json")), status_code=status, headers={"Retry-After": "0"}
            ),
            page([record("mock_events", 0)]),
        ],
    )
    output = read_stream(CONNECTOR, "mock_events", config())
    assert not output.errors
    assert len(output.records) == 1


def test_authentication_failure_is_terminal(http_mocker: HttpMocker) -> None:
    http_mocker.get(
        HttpRequest(f"{BASE_URL}/streams/mock_events", query_params={}),
        HttpResponse(body=json.dumps(load_fixture(__file__, "error.json")), status_code=401),
    )
    output = read_stream(CONNECTOR, "mock_events", config(), expecting_exception=True)
    assert output.errors
    assert not output.records


@freeze_time("2026-02-01")
def test_incremental_state_resumes_from_checkpoint(http_mocker: HttpMocker) -> None:
    url = f"{BASE_URL}/streams/mock_updates"
    http_mocker.get(HttpRequest(url, query_params=query("mock_updates")), page([record("mock_updates", 1)]))
    first = read_stream(CONNECTOR, "mock_updates", config(), sync_mode=SyncMode.incremental)
    assert first.state_messages
    final_state = first.state_messages[-1].state.stream.stream_state.__dict__
    assert final_state["last_updated_at"] == "2026-01-01T00:00:01Z"
    http_mocker.get(
        HttpRequest(url, query_params={"cursor": "2026-01-01T00:00:01Z"}), page([record("mock_updates", 2)])
    )
    resumed = read_stream(
        CONNECTOR, "mock_updates", config(), state=[first.state_messages[-1].state], sync_mode=SyncMode.incremental
    )
    assert not resumed.errors
    assert [row.record.data["id"] for row in resumed.records] == ["mock_updates-2"]


def test_check_uses_empty_probe_and_discovery_has_two_data_streams(http_mocker: HttpMocker) -> None:
    source = get_source(CONNECTOR, config())
    logger = logging.getLogger("mock-test")
    catalog = source.discover(logger, config())
    assert {stream.name for stream in catalog.streams} == {*STREAMS, "mock_probe"}
    http_mocker.get(HttpRequest(f"{BASE_URL}/probe", query_params={}), page([]))
    assert source.check(logger, config()).status == Status.SUCCEEDED
