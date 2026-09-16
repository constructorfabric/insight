from __future__ import annotations

import json
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
    records = list(stream_for(client).read_records(SyncMode.full_refresh))
    after = datetime.now(UTC).date().isoformat()

    method, path, params = client.calls[0]
    assert (method, path) == ("GET", PATH)
    assert params["start"] == "2024-01-01"
    assert before <= params["end"] <= after
    assert params["filter"] == "off"
    assert len(records) == 1
    assert records[0]["window_start"] == params["start"]
    assert records[0]["window_end"] == params["end"]
    assert json.loads(records[0]["entries_json"]) == []
    validate(records[0], stream_for(client).get_json_schema())


def test_time_off_and_holidays_share_one_lossless_envelope() -> None:
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

    assert len(records) == 1
    assert records == list(stream.read_records(SyncMode.full_refresh))
    record = records[0]
    assert record["tenant_id"] == TENANT
    assert record["source_id"] == SOURCE
    assert json.loads(record["entries_json"]) == rows
    assert all("unique_key" not in row for row in rows)
    validate(record, stream.get_json_schema())


def test_empty_refresh_keeps_the_same_snapshot_key() -> None:
    rows = [{"id": 7, "type": "holiday", "start": "2024-05-01"}]
    stream = stream_for(FakeClient({PATH: rows}))
    [first] = list(stream.read_records(SyncMode.full_refresh))
    rows.clear()
    [second] = list(stream.read_records(SyncMode.full_refresh))

    assert first["unique_key"] == second["unique_key"]
    assert len(json.loads(first["entries_json"])) == 1
    assert json.loads(second["entries_json"]) == []


def test_snapshot_keys_isolate_tenants_and_sources() -> None:
    keys = set()
    for tenant, source in [("a", "b-c"), ("a-b", "c"), ("a", "c"), ("a-b", "b-c")]:
        stream = WhosOutStream(FakeClient({PATH: []}), tenant, source, "2024-01-01")
        [record] = list(stream.read_records(SyncMode.full_refresh))
        keys.add(record["unique_key"])

    assert len(keys) == 4


def test_history_window_changes_do_not_change_snapshot_key() -> None:
    keys = set()
    for start in ["2024-01-01", "2025-01-01"]:
        stream = WhosOutStream(FakeClient({PATH: []}), TENANT, SOURCE, start)
        [record] = list(stream.read_records(SyncMode.full_refresh))
        keys.add(record["unique_key"])
        assert record["window_start"] == start

    assert len(keys) == 1


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
def test_raw_envelope_does_not_silently_discard_entries(row: object) -> None:
    records = list(stream_for(FakeClient({PATH: [row]})).read_records(SyncMode.full_refresh))
    assert len(records) == 1
    assert json.loads(records[0]["entries_json"]) == [row]


@pytest.mark.parametrize("start_date", [None, "2024-01-01"])
def test_source_passes_shared_history_start(start_date: str | None, monkeypatch: pytest.MonkeyPatch) -> None:
    client = FakeClient({PATH: []})
    monkeypatch.setattr("source_bamboohr.source._client", lambda _: client)
    config = {"insight_tenant_id": TENANT, "insight_source_id": SOURCE, "bamboohr_start_date": start_date}
    stream = next(stream for stream in SourceBamboohr().streams(config) if stream.name == "whos_out")

    assert not stream.supports_incremental
    assert len(list(stream.read_records(SyncMode.full_refresh))) == 1
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
