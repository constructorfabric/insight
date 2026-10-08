from __future__ import annotations

import freezegun
import pytest
from airbyte_cdk.models import SyncMode
from config import (
    FROZEN_NOW,
    LAUNCH_ERRORS_URL,
    AllureConfigBuilder,
    api_request,
    error,
    mock_launch_window,
    mock_projects,
    page,
    paged,
)
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "launch_errors"
_LAUNCH_MODIFIED_MS = 1781519400123


def _launch(launch_id: int, project_id: int, modified_ms: int = _LAUNCH_MODIFIED_MS) -> dict:
    return load_fixture(__file__, "launch.json", id=launch_id, projectId=project_id, lastModifiedDate=modified_ms)


def _launch_error(error_id: int, launch_id: int) -> dict:
    return load_fixture(__file__, "launch_error.json", id=error_id, launchId=launch_id)


def _params(launch_id: int, page_index: int | None = None) -> dict:
    return paged({"launchId": str(launch_id)}, page_index)


def _mock_errors(http_mocker: HttpMocker, launch_id: int, response: HttpResponse | list) -> HttpRequest:
    request = api_request(LAUNCH_ERRORS_URL, _params(launch_id))
    http_mocker.get(request, response)

    return request


def _read(config: dict, state: list | None = None):
    return read_stream(_CONNECTOR, _STREAM, config, state=state, sync_mode=SyncMode.incremental)


def _final_state(output) -> dict:
    return output.state_messages[-1].state.stream.stream_state.__dict__


@freezegun.freeze_time(FROZEN_NOW)
def test_records_carry_launch_and_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_errors(http_mocker, 101, page([_launch_error(81, 101)]))

    output = _read(config)

    record = output.records[0].record.data
    assert record["message"] == "Upload of results archive failed"
    assert record["launch_id"] == 101
    assert record["project_id"] == 7
    assert record["lastModifiedDate"] == _LAUNCH_MODIFIED_MS


@freezegun.freeze_time(FROZEN_NOW)
def test_records_stamped_with_tenant_source_and_unique_key(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_errors(http_mocker, 101, page([_launch_error(81, 101)]))

    output = _read(config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"
    assert record["unique_key"] == "test-tenant-test-source-81"


@freezegun.freeze_time(FROZEN_NOW)
def test_pagination_within_launch_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_errors(http_mocker, 101, page([_launch_error(81, 101)], number=0, last=False))
    http_mocker.get(api_request(LAUNCH_ERRORS_URL, _params(101, page_index=1)), page([_launch_error(82, 101)], number=1))

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [81, 82]


@freezegun.freeze_time(FROZEN_NOW)
def test_launch_deleted_between_list_and_read_is_skipped(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7), _launch(102, 7)]))
    _mock_errors(http_mocker, 101, error(404))
    _mock_errors(http_mocker, 102, page([_launch_error(82, 102)]))

    output = _read(config)

    assert not output.errors
    assert [r.record.data["launch_id"] for r in output.records] == [102]


@freezegun.freeze_time(FROZEN_NOW)
def test_launch_without_errors_emits_nothing(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_errors(http_mocker, 101, page([]))

    output = _read(config)

    assert not output.errors
    assert not output.records


@freezegun.freeze_time(FROZEN_NOW)
def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_errors(http_mocker, 101, page([_launch_error(81, 101)]))

    output = _read(config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(FROZEN_NOW)
def test_parent_launch_cursor_persisted_in_state(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_errors(http_mocker, 101, page([_launch_error(81, 101)]))

    output = _read(config)

    launches_state = _final_state(output)["parent_state"]["_launches"]
    cursors = {s["partition"]["project_id"]: s["cursor"]["lastModifiedDate"] for s in launches_state["states"]}
    assert cursors == {7: str(_LAUNCH_MODIFIED_MS)}


@pytest.mark.usefixtures("slept")
@freezegun.freeze_time(FROZEN_NOW)
def test_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_errors(http_mocker, 101, [error(500), page([_launch_error(81, 101)])])

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [81]
