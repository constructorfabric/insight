from __future__ import annotations

import json

import freezegun
import pytest
from airbyte_cdk.models import SyncMode
from config import (
    FROZEN_NOW,
    TEST_CASE_SEARCH_URL,
    AllureConfigBuilder,
    api_request,
    case_overview_url,
    case_search_params,
    error,
    mock_case_search,
    page,
)
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "test_cases"


def _found(test_case_id: int, project_id: int, modified_ms: int) -> dict:
    return load_fixture(
        __file__, "test_case_search.json", id=test_case_id, projectId=project_id, lastModifiedDate=modified_ms
    )


def _overview(test_case_id: int, project_id: int = 7, modified_ms: int = 1781519400123) -> HttpResponse:
    body = load_fixture(__file__, "test_case.json", id=test_case_id, projectId=project_id, lastModifiedDate=modified_ms)

    return HttpResponse(body=json.dumps(body), status_code=200)


def _mock_overview(http_mocker: HttpMocker, test_case_id: int, response: HttpResponse | list) -> HttpRequest:
    request = api_request(case_overview_url(test_case_id), {})
    http_mocker.get(request, response)

    return request


def _read(config: dict, state: list | None = None):
    return read_stream(_CONNECTOR, _STREAM, config, state=state, sync_mode=SyncMode.incremental)


def _final_state(output) -> dict:
    return output.state_messages[-1].state.stream.stream_state.__dict__


def _parent_cursors_by_project(state: dict) -> dict:
    search_state = state["parent_state"]["_test_cases"]

    return {s["partition"]["project_id"]: s["cursor"]["lastModifiedDate"] for s in search_state["states"]}


def _first_sync_state(http_mocker: HttpMocker, config: dict) -> list:
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)]))
    _mock_overview(http_mocker, 9001, _overview(9001))

    first = _read(config)

    return [m.state for m in first.state_messages][-1:]


@freezegun.freeze_time(FROZEN_NOW)
def test_first_sync_searches_every_test_case_since_2000(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    search = mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)]))
    _mock_overview(http_mocker, 9001, _overview(9001))

    output = _read(config)

    assert not output.errors
    http_mocker.assert_number_of_calls(search, 1)


@freezegun.freeze_time(FROZEN_NOW)
def test_one_overview_request_per_test_case_found(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123), _found(9002, 7, 1781942400456)]))
    mock_case_search(http_mocker, 12, page([_found(9101, 12, 1778403600000)]))
    overviews = [
        _mock_overview(http_mocker, 9001, _overview(9001)),
        _mock_overview(http_mocker, 9002, _overview(9002)),
        _mock_overview(http_mocker, 9101, _overview(9101, project_id=12)),
    ]

    output = _read(config)

    assert not output.errors
    assert sorted((r.record.data["projectId"], r.record.data["id"]) for r in output.records) == [
        (7, 9001),
        (7, 9002),
        (12, 9101),
    ]
    for overview in overviews:
        http_mocker.assert_number_of_calls(overview, 1)


@freezegun.freeze_time(FROZEN_NOW)
def test_record_keeps_every_custom_field_value(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)]))
    _mock_overview(http_mocker, 9001, _overview(9001))

    output = _read(config)

    custom_fields = output.records[0].record.data["customFields"]
    assert [(v["customField"]["name"], v["name"]) for v in custom_fields] == [
        ("Epic", "Exams"),
        ("Feature", "Exam start"),
        ("Story", "Happy path"),
        ("Story", "Retry"),
        ("SubProject", "Web"),
    ]


@freezegun.freeze_time(FROZEN_NOW)
def test_records_stamped_with_tenant_source_and_unique_key(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)]))
    _mock_overview(http_mocker, 9001, _overview(9001))

    output = _read(config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"
    assert record["unique_key"] == "test-tenant-test-source-9001"


@freezegun.freeze_time(FROZEN_NOW)
def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)]))
    _mock_overview(http_mocker, 9001, _overview(9001))

    output = _read(config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freezegun.freeze_time(FROZEN_NOW)
def test_project_without_test_cases_emits_nothing(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, page([]))

    output = _read(config)

    assert not output.errors
    assert not output.records


@freezegun.freeze_time(FROZEN_NOW)
def test_search_pages_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)], number=0, last=False))
    http_mocker.get(
        api_request(TEST_CASE_SEARCH_URL, case_search_params(7, page_index=1)),
        page([_found(9002, 7, 1781942400456)], number=1, last=True),
    )
    _mock_overview(http_mocker, 9001, _overview(9001))
    _mock_overview(http_mocker, 9002, _overview(9002))

    output = _read(config)

    assert not output.errors
    assert sorted(r.record.data["id"] for r in output.records) == [9001, 9002]


@freezegun.freeze_time(FROZEN_NOW)
def test_search_cursor_persisted_per_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)]))
    _mock_overview(http_mocker, 9001, _overview(9001))

    output = _read(config)

    final_state = _final_state(output)
    assert "__ab_no_cursor_state_message" not in final_state
    assert _parent_cursors_by_project(final_state) == {7: "1781519400123"}


@freezegun.freeze_time(FROZEN_NOW)
def test_resumed_sync_fetches_only_test_cases_modified_since_cursor_minus_two_days(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    state = _first_sync_state(http_mocker, config)

    resume_mocker = HttpMocker()
    with resume_mocker:
        search = api_request(TEST_CASE_SEARCH_URL, case_search_params(7, 1781346600123))
        resume_mocker.get(search, page([_found(9002, 7, 1782388800789)]))
        overview = _mock_overview(resume_mocker, 9002, _overview(9002, modified_ms=1782388800789))

        second = _read(config, state=state)

        assert not second.errors
        assert [r.record.data["id"] for r in second.records] == [9002]
        resume_mocker.assert_number_of_calls(search, 1)
        resume_mocker.assert_number_of_calls(overview, 1)
        assert _parent_cursors_by_project(_final_state(second)) == {7: "1782388800789"}


@freezegun.freeze_time(FROZEN_NOW)
def test_rate_limited_overview_is_retried(http_mocker: HttpMocker, slept: list) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, page([_found(9001, 7, 1781519400123)]))
    _mock_overview(http_mocker, 9001, [error(429, {"Retry-After": "5"}), _overview(9001)])

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [9001]
    assert len(slept) == 1


@pytest.mark.usefixtures("slept")
@freezegun.freeze_time(FROZEN_NOW)
def test_search_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_case_search(http_mocker, 7, [error(504), page([_found(9001, 7, 1781519400123)])])
    _mock_overview(http_mocker, 9001, _overview(9001))

    output = _read(config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [9001]
