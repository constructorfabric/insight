from __future__ import annotations

import freezegun
import pytest
from airbyte_cdk.models import SyncMode
from config import (
    FROZEN_NOW,
    LAUNCHES_URL,
    NOW_MS,
    AllureConfigBuilder,
    api_request,
    error,
    launch_params,
    mock_launch_window,
    mock_token,
    page,
)
from connector_tests import HttpMocker, assert_records_conform, get_source, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "launches"


def _launch(launch_id: int, project_id: int, modified_ms: int) -> dict:
    return load_fixture(__file__, "launch.json", id=launch_id, projectId=project_id, lastModifiedDate=modified_ms)


def _read(config: dict, state: list | None = None):
    return read_stream(_CONNECTOR, _STREAM, config, state=state, sync_mode=SyncMode.incremental)


def _final_state(output) -> dict:
    return output.state_messages[-1].state.stream.stream_state.__dict__


def _cursors_by_project(state: dict) -> dict:
    return {s["partition"]["project_id"]: s["cursor"]["lastModifiedDate"] for s in state["states"]}


@freezegun.freeze_time(FROZEN_NOW)
def test_first_sync_requests_one_open_ended_window_from_ninety_days_back(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    window = mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [101]
    http_mocker.assert_number_of_calls(window, 1)


@freezegun.freeze_time(FROZEN_NOW)
def test_one_request_set_per_configured_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_token(http_mocker)
    requests = [
        mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)])),
        mock_launch_window(http_mocker, 12, page([_launch(201, 12, 1778403600000)])),
    ]

    output = _read(config)

    assert not output.errors
    assert sorted(r.record.data["id"] for r in output.records) == [101, 201]
    for request in requests:
        http_mocker.assert_number_of_calls(request, 1)


@freezegun.freeze_time(FROZEN_NOW)
def test_records_stamped_with_tenant_source_and_unique_key(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))

    output = _read(config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"
    assert record["unique_key"] == "test-tenant-test-source-101"


@freezegun.freeze_time(FROZEN_NOW)
def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))

    output = _read(config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(FROZEN_NOW)
def test_empty_windows_emit_nothing(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7)

    output = _read(config)

    assert not output.errors
    assert not output.records


@freezegun.freeze_time(FROZEN_NOW)
def test_pagination_within_window_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)], number=0, last=False))
    http_mocker.get(
        api_request(LAUNCHES_URL, launch_params(7, page_index=1)),
        page([_launch(102, 7, 1781942400456)], number=1, last=True),
    )

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [101, 102]


@freezegun.freeze_time(FROZEN_NOW)
def test_launch_modified_after_sync_start_stays_in_window_and_becomes_cursor(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    modified_mid_read_ms = NOW_MS + 60000
    mock_launch_window(
        http_mocker, 7, page([_launch(101, 7, 1781519400123), _launch(102, 7, 1781942400456)], number=0, last=False)
    )
    http_mocker.get(
        api_request(LAUNCHES_URL, launch_params(7, page_index=1)),
        page([_launch(103, 7, modified_mid_read_ms)], number=1, last=True),
    )

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [101, 102, 103]
    assert _cursors_by_project(_final_state(output)) == {7: str(modified_mid_read_ms)}


@freezegun.freeze_time(FROZEN_NOW)
def test_state_tracks_latest_modified_launch_per_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123), _launch(102, 7, 1781346600123)]))
    mock_launch_window(http_mocker, 12, page([_launch(201, 12, 1778403600000)]))

    output = _read(config)

    assert _cursors_by_project(_final_state(output)) == {7: "1781519400123", 12: "1778403600000"}


@freezegun.freeze_time(FROZEN_NOW)
def test_resumed_sync_starts_each_project_from_its_cursor_minus_two_day_lookback(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    mock_launch_window(http_mocker, 12, page([_launch(201, 12, 1781942400456)]))

    first = _read(config)

    state = [m.state for m in first.state_messages][-1:]

    resume_mocker = HttpMocker()
    with resume_mocker:
        mock_token(resume_mocker)
        project_7 = api_request(LAUNCHES_URL, launch_params(7, 1781346600123))
        project_12 = api_request(LAUNCHES_URL, launch_params(12, 1781769600456))
        resume_mocker.get(project_7, page([]))
        resume_mocker.get(project_12, page([]))

        second = _read(config, state=state)

        assert not second.errors
        resume_mocker.assert_number_of_calls(project_7, 1)
        resume_mocker.assert_number_of_calls(project_12, 1)


@freezegun.freeze_time(FROZEN_NOW)
def test_rate_limited_request_is_retried(http_mocker: HttpMocker, slept: list) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7, [error(429, {"Retry-After": "5"}), page([_launch(101, 7, 1781519400123)])])

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [101]
    assert len(slept) == 1


@pytest.mark.usefixtures("slept")
@freezegun.freeze_time(FROZEN_NOW)
def test_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_token(http_mocker)
    mock_launch_window(http_mocker, 7, [error(502), page([_launch(101, 7, 1781519400123)])])

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [101]


def test_empty_project_list_is_rejected() -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", []).build()

    with pytest.raises(ValueError, match="should be non-empty"):
        get_source(_CONNECTOR, config)
