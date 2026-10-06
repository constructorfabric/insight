from __future__ import annotations

import freezegun
import pytest
from airbyte_cdk.models import SyncMode
from config import (
    FROZEN_NOW,
    LAUNCHES_URL,
    TEST_RESULTS_URL,
    AllureConfigBuilder,
    api_request,
    error,
    launch_params,
    mock_launch_window,
    page,
    result_params,
)
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "test_results"


def _launch(launch_id: int, project_id: int, modified_ms: int) -> dict:
    return load_fixture(__file__, "launch.json", id=launch_id, projectId=project_id, lastModifiedDate=modified_ms)


def _result(result_id: int, launch_id: int, modified_ms: int = 1781519100000) -> dict:
    return load_fixture(__file__, "test_result.json", id=result_id, launchId=launch_id, lastModifiedDate=modified_ms)


def _mock_results(http_mocker: HttpMocker, launch_id: int, response: HttpResponse | list) -> HttpRequest:
    request = api_request(TEST_RESULTS_URL, result_params(launch_id))
    http_mocker.get(request, response)

    return request


def _read(config: dict, state: list | None = None):
    return read_stream(_CONNECTOR, _STREAM, config, state=state, sync_mode=SyncMode.incremental)


def _final_state(output) -> dict:
    return output.state_messages[-1].state.stream.stream_state.__dict__


def _parent_cursors_by_project(state: dict) -> dict:
    launches_state = state["parent_state"]["_launches"]

    return {s["partition"]["project_id"]: s["cursor"]["lastModifiedDate"] for s in launches_state["states"]}


def _first_sync_state(http_mocker: HttpMocker, config: dict) -> list:
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, page([_result(5001, 101)]))

    first = _read(config)

    return [m.state for m in first.state_messages][-1:]


@freezegun.freeze_time(FROZEN_NOW)
def test_one_request_per_parent_launch_with_launch_id(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123), _launch(102, 7, 1781942400456)]))
    mock_launch_window(http_mocker, 12, page([_launch(201, 12, 1778403600000)]))
    children = [
        _mock_results(http_mocker, 101, page([_result(5001, 101)])),
        _mock_results(http_mocker, 102, page([_result(5002, 102)])),
        _mock_results(http_mocker, 201, page([_result(5003, 201)])),
    ]

    output = _read(config)

    assert not output.errors
    assert sorted((r.record.data["launchId"], r.record.data["id"]) for r in output.records) == [
        (101, 5001),
        (102, 5002),
        (201, 5003),
    ]
    for child in children:
        http_mocker.assert_number_of_calls(child, 1)


@freezegun.freeze_time(FROZEN_NOW)
def test_configured_start_date_applies_to_parent_launches(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_start_date", "2025-01-01").build()
    parent = api_request(LAUNCHES_URL, launch_params(7, 1735689600000))
    http_mocker.get(parent, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, page([_result(5001, 101)]))

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [5001]
    http_mocker.assert_number_of_calls(parent, 1)


@freezegun.freeze_time(FROZEN_NOW)
def test_records_stamped_with_tenant_source_and_unique_key(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, page([_result(5001, 101)]))

    output = _read(config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"
    assert record["unique_key"] == "test-tenant-test-source-5001"


@freezegun.freeze_time(FROZEN_NOW)
def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, page([_result(5001, 101)]))

    output = _read(config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(FROZEN_NOW)
def test_launch_without_results_emits_nothing(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, page([]))

    output = _read(config)

    assert not output.errors
    assert not output.records


@freezegun.freeze_time(FROZEN_NOW)
def test_pagination_within_launch_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, page([_result(5001, 101)], number=0, last=False))
    http_mocker.get(
        api_request(TEST_RESULTS_URL, result_params(101, page_index=1)), page([_result(5002, 101)], number=1, last=True)
    )

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [5001, 5002]


@freezegun.freeze_time(FROZEN_NOW)
def test_parent_pages_through_its_open_window_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)], number=0, last=False))
    http_mocker.get(
        api_request(LAUNCHES_URL, launch_params(7, page_index=1)),
        page([_launch(102, 7, 1781942400456)], number=1, last=True),
    )
    _mock_results(http_mocker, 101, page([_result(5001, 101)]))
    _mock_results(http_mocker, 102, page([_result(5002, 102)]))

    output = _read(config)

    assert not output.errors
    assert sorted(r.record.data["id"] for r in output.records) == [5001, 5002]


@freezegun.freeze_time(FROZEN_NOW)
def test_parent_launch_cursor_persisted_in_state(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, page([_result(5001, 101)]))

    output = _read(config)

    final_state = _final_state(output)
    assert "__ab_no_cursor_state_message" not in final_state
    assert _parent_cursors_by_project(final_state) == {7: "1781519400123"}


@freezegun.freeze_time(FROZEN_NOW)
def test_resumed_sync_enumerates_only_launches_modified_since_saved_parent_state(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    state = _first_sync_state(http_mocker, config)

    resume_mocker = HttpMocker()
    with resume_mocker:
        parent = api_request(LAUNCHES_URL, launch_params(7, 1781346600123))
        resume_mocker.get(parent, page([_launch(202, 7, 1782388800789)]))
        child = _mock_results(resume_mocker, 202, page([_result(5002, 202)]))

        second = _read(config, state=state)

        assert not second.errors
        assert [r.record.data["id"] for r in second.records] == [5002]
        resume_mocker.assert_number_of_calls(parent, 1)
        resume_mocker.assert_number_of_calls(child, 1)
        assert _parent_cursors_by_project(_final_state(second)) == {7: "1782388800789"}


@freezegun.freeze_time(FROZEN_NOW)
def test_resumed_sync_emits_results_older_than_saved_cursor(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    state = _first_sync_state(http_mocker, config)

    resume_mocker = HttpMocker()
    with resume_mocker:
        resume_mocker.get(
            api_request(LAUNCHES_URL, launch_params(7, 1781346600123)), page([_launch(202, 7, 1782388800789)])
        )
        _mock_results(resume_mocker, 202, page([_result(5002, 202, modified_ms=1767225600000)]))

        second = _read(config, state=state)

        assert not second.errors
        assert [r.record.data["id"] for r in second.records] == [5002]


@freezegun.freeze_time(FROZEN_NOW)
def test_rate_limited_request_is_retried(http_mocker: HttpMocker, slept: list) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, [error(429, {"Retry-After": "5"}), page([_result(5001, 101)])])

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [5001]
    assert len(slept) == 1


@pytest.mark.usefixtures("slept")
@freezegun.freeze_time(FROZEN_NOW)
def test_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, page([_launch(101, 7, 1781519400123)]))
    _mock_results(http_mocker, 101, [error(500), page([_result(5001, 101)])])

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [5001]


@pytest.mark.usefixtures("slept")
@freezegun.freeze_time(FROZEN_NOW)
def test_parent_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_launch_window(http_mocker, 7, [error(504), page([_launch(101, 7, 1781519400123)])])
    _mock_results(http_mocker, 101, page([_result(5001, 101)]))

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [5001]
