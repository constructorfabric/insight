from __future__ import annotations

import logging
from datetime import UTC, datetime

import pytest
from airbyte_cdk.models import ConfiguredAirbyteCatalog, ConfiguredAirbyteStream, DestinationSyncMode, SyncMode, Type
from conftest import SOURCE, TENANT, FakeClient
from jsonschema import validate
from source_bamboohr.client import BambooHrApiError, BambooHrAuthError
from source_bamboohr.source import SourceBamboohr
from source_bamboohr.streams.leave_requests import LeaveRequestsStream
from source_bamboohr.streams.whos_out import WhosOutStream

PATH = "time_off/whos_out"


def stream_for(client: FakeClient) -> WhosOutStream:
    return WhosOutStream(client=client, tenant_id=TENANT, source_id=SOURCE, start_date="2024-01-01")


def test_history_window_ignores_saved_employee_filter() -> None:
    client = FakeClient({PATH: []})
    before = datetime.now(UTC).date().isoformat()
    assert list(stream_for(client).read_records(SyncMode.full_refresh)) == []
    after = datetime.now(UTC).date().isoformat()

    method, path, params = client.calls[0]
    assert (method, path) == ("GET", PATH)
    assert params["start"] == "2024-01-01"
    assert before <= params["end"] <= after
    assert params["filter"] == "off"


def test_time_off_and_holidays_keep_payloads_and_distinct_occurrence_keys() -> None:
    rows = [
        {
            "id": 7,
            "type": "timeOff",
            "employeeId": 42,
            "name": "Example Person",
            "start": "2024-05-01",
            "end": "2024-05-02",
        },
        {"id": 7, "type": "holiday", "name": "Example Holiday", "start": "2024-05-01", "end": "2024-05-01"},
        {"id": 7, "type": "holiday", "name": "Example Holiday", "start": "2025-05-01", "end": "2025-05-01"},
    ]
    stream = stream_for(FakeClient({PATH: rows}))
    records = list(stream.read_records(SyncMode.full_refresh))

    assert len(records) == len(rows)
    assert len({record["unique_key"] for record in records}) == len(rows)
    assert records == list(stream.read_records(SyncMode.full_refresh))
    for row, record in zip(rows, records, strict=True):
        assert record == {**row, "tenant_id": TENANT, "source_id": SOURCE, "unique_key": record["unique_key"]}
        assert "unique_key" not in row
        validate(record, stream.get_json_schema())


def test_forbidden_whos_out_warns_without_failing(caplog: pytest.LogCaptureFixture) -> None:
    client = FakeClient({PATH: BambooHrAuthError(403, "url", "private response detail")})
    with caplog.at_level(logging.WARNING, logger="airbyte"):
        assert list(stream_for(client).read_records(SyncMode.full_refresh)) == []

    assert "Skipping BambooHR whos_out" in caplog.text
    assert "403" in caplog.text
    assert "private response detail" not in caplog.text


@pytest.mark.parametrize("status", [401, 404, 429, 500])
def test_other_api_failures_remain_errors(status: int) -> None:
    error = BambooHrApiError(status, "url", "failure")
    with pytest.raises(BambooHrApiError) as caught:
        list(stream_for(FakeClient({PATH: error})).read_records(SyncMode.full_refresh))
    assert caught.value is error


def test_transport_failures_remain_errors() -> None:
    with pytest.raises(OSError, match="unreachable"):
        list(stream_for(FakeClient({PATH: OSError("unreachable")})).read_records(SyncMode.full_refresh))


def test_non_list_response_is_an_error() -> None:
    with pytest.raises(TypeError, match="not a list"):
        list(stream_for(FakeClient({PATH: {"entries": []}})).read_records(SyncMode.full_refresh))


@pytest.mark.parametrize("row", [None, "invalid", {}, {"id": "", "type": "holiday", "start": "2024-05-01"}])
def test_unkeyable_rows_are_skipped(row: object) -> None:
    assert list(stream_for(FakeClient({PATH: [row]})).read_records(SyncMode.full_refresh)) == []


@pytest.mark.parametrize("start_date", [None, "2024-01-01"])
def test_source_passes_shared_history_start(start_date: str | None, monkeypatch: pytest.MonkeyPatch) -> None:
    client = FakeClient({PATH: []})
    monkeypatch.setattr("source_bamboohr.source._client", lambda _: client)
    config = {"insight_tenant_id": TENANT, "insight_source_id": SOURCE, "bamboohr_start_date": start_date}
    stream = next(stream for stream in SourceBamboohr().streams(config) if stream.name == "whos_out")

    assert not stream.supports_incremental
    assert list(stream.read_records(SyncMode.full_refresh)) == []
    assert client.calls[0][2]["start"] == (start_date or "2020-01-01")


def test_forbidden_whos_out_does_not_stop_other_source_streams(monkeypatch: pytest.MonkeyPatch) -> None:
    client = FakeClient({PATH: BambooHrAuthError(403, "url", "disabled"), "meta/fields": [{"id": 9}]})
    monkeypatch.setattr("source_bamboohr.source._client", lambda _: client)
    config = {"insight_tenant_id": TENANT, "insight_source_id": SOURCE}
    source = SourceBamboohr()
    streams = {stream.name: stream for stream in source.streams(config)}
    catalog = ConfiguredAirbyteCatalog(
        streams=[
            ConfiguredAirbyteStream(
                stream=streams[name].as_airbyte_stream(),
                sync_mode=SyncMode.full_refresh,
                destination_sync_mode=DestinationSyncMode.append,
            )
            for name in ("whos_out", "meta_fields")
        ]
    )

    messages = list(source.read(logging.getLogger("airbyte"), config, catalog))
    records = [message.record for message in messages if message.type == Type.RECORD]
    assert len(records) == 1
    assert records[0].stream == "meta_fields"
    assert records[0].data["id"] == 9
    assert [path for _, path, _ in client.calls] == [PATH, "meta/fields"]


def test_forbidden_leave_requests_still_fails() -> None:
    client = FakeClient({"time_off/requests": BambooHrAuthError(403, "url", "forbidden")})
    stream = LeaveRequestsStream(client=client, tenant_id=TENANT, source_id=SOURCE, start_date="2024-01-01")
    with pytest.raises(BambooHrAuthError):
        list(stream.read_records(SyncMode.full_refresh))
