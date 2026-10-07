from __future__ import annotations

import json

import freezegun
import pytest
from airbyte_cdk.models import SyncMode
from config import (
    FROZEN_NOW,
    AllureConfigBuilder,
    api_request,
    error,
    launch_env_url,
    mock_launch_window,
    mock_projects,
    page,
)
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "launch_environment"
_LAUNCH_MODIFIED_MS = 1781519400123


def _launch(launch_id: int, project_id: int, modified_ms: int = _LAUNCH_MODIFIED_MS) -> dict:
    return load_fixture(__file__, "launch.json", id=launch_id, projectId=project_id, lastModifiedDate=modified_ms)


def _value(value_id: int, value: str, variable_id: int, variable_name: str) -> dict:
    variable = {**load_fixture(__file__, "launch_env_value.json")["variable"], "id": variable_id, "name": variable_name}

    return load_fixture(__file__, "launch_env_value.json", id=value_id, name=value, variable=variable)


def _env(values: list[dict]) -> HttpResponse:
    return HttpResponse(body=json.dumps(values), status_code=200)


def _mock_env(http_mocker: HttpMocker, launch_id: int, response: HttpResponse | list) -> HttpRequest:
    request = api_request(launch_env_url(launch_id), {})
    http_mocker.get(request, response)

    return request


def _read(config: dict, state: list | None = None):
    return read_stream(_CONNECTOR, _STREAM, config, state=state, sync_mode=SyncMode.incremental)


def _final_state(output) -> dict:
    return output.state_messages[-1].state.stream.stream_state.__dict__


def _parent_cursors_by_project(state: dict) -> dict:
    launches_state = state["parent_state"]["_launches"]

    return {s["partition"]["project_id"]: s["cursor"]["lastModifiedDate"] for s in launches_state["states"]}


@freezegun.freeze_time(FROZEN_NOW)
def test_one_record_per_value_carrying_launch_project_and_variable(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_env(http_mocker, 101, _env([_value(61, "dev", 71, "Backend Stand"), _value(62, "chromium", 72, "Browser")]))

    output = _read(config)

    records = [r.record.data for r in output.records]
    assert [(r["variable"]["name"], r["name"]) for r in records] == [("Backend Stand", "dev"), ("Browser", "chromium")]
    assert {r["launch_id"] for r in records} == {101}
    assert {r["project_id"] for r in records} == {7}
    assert {r["lastModifiedDate"] for r in records} == {_LAUNCH_MODIFIED_MS}


@freezegun.freeze_time(FROZEN_NOW)
def test_records_stamped_with_tenant_source_and_launch_value_key(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_env(http_mocker, 101, _env([_value(61, "dev", 71, "Backend Stand")]))

    output = _read(config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"
    assert record["unique_key"] == "test-tenant-test-source-101-61"


@freezegun.freeze_time(FROZEN_NOW)
def test_two_values_of_one_variable_give_two_records(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_env(http_mocker, 101, _env([_value(61, "dev", 71, "Backend Stand"), _value(63, "test", 71, "Backend Stand")]))

    output = _read(config)

    keys = [r.record.data["unique_key"] for r in output.records]
    assert keys == ["test-tenant-test-source-101-61", "test-tenant-test-source-101-63"]


@freezegun.freeze_time(FROZEN_NOW)
def test_launch_deleted_between_list_and_read_is_skipped(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7), _launch(102, 7)]))
    _mock_env(http_mocker, 101, error(404))
    _mock_env(http_mocker, 102, _env([_value(61, "dev", 71, "Backend Stand")]))

    output = _read(config)

    assert not output.errors
    assert [r.record.data["launch_id"] for r in output.records] == [102]


@freezegun.freeze_time(FROZEN_NOW)
def test_launch_without_environment_emits_nothing(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_env(http_mocker, 101, _env([]))

    output = _read(config)

    assert not output.errors
    assert not output.records


@freezegun.freeze_time(FROZEN_NOW)
def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_env(http_mocker, 101, _env([_value(61, "dev", 71, "Backend Stand")]))

    output = _read(config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(FROZEN_NOW)
def test_parent_launch_cursor_persisted_in_state(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_env(http_mocker, 101, _env([_value(61, "dev", 71, "Backend Stand")]))

    output = _read(config)

    final_state = _final_state(output)
    assert "__ab_no_cursor_state_message" not in final_state
    assert _parent_cursors_by_project(final_state) == {7: str(_LAUNCH_MODIFIED_MS)}


@pytest.mark.usefixtures("slept")
@freezegun.freeze_time(FROZEN_NOW)
def test_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_launch_window(http_mocker, 7, page([_launch(101, 7)]))
    _mock_env(http_mocker, 101, [error(500), _env([_value(61, "dev", 71, "Backend Stand")])])

    output = _read(config)

    assert not output.errors
    assert [r.record.data["name"] for r in output.records] == ["dev"]
