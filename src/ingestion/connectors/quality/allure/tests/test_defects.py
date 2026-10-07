from __future__ import annotations

import json

import pytest
from config import (
    DEFECTS_URL,
    AllureConfigBuilder,
    api_request,
    defect_url,
    error,
    mock_defect_list,
    mock_projects,
    page,
    paged,
)
from connector_tests import HttpMocker, HttpResponse, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "defects"


def _row(defect_id: int) -> dict:
    return load_fixture(__file__, "defect_row.json", id=defect_id)


def _detail(defect_id: int, project_id: int, name: str) -> HttpResponse:
    body = load_fixture(__file__, "defect.json", id=defect_id, projectId=project_id, name=name)

    return HttpResponse(body=json.dumps(body), status_code=200)


def _mock_detail(http_mocker: HttpMocker, defect_id: int, response: HttpResponse | list) -> None:
    http_mocker.get(api_request(defect_url(defect_id), {}), response)


def test_one_detail_record_per_listed_defect_across_projects(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201), _row(1202)]))
    mock_defect_list(http_mocker, 12, page([_row(1301)]))
    _mock_detail(http_mocker, 1201, _detail(1201, 7, "Login page times out"))
    _mock_detail(http_mocker, 1202, _detail(1202, 7, "Report export hangs"))
    _mock_detail(http_mocker, 1301, _detail(1301, 12, "Camera check fails"))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted((r.record.data["projectId"], r.record.data["name"]) for r in output.records) == [
        (7, "Login page times out"),
        (7, "Report export hangs"),
        (12, "Camera check fails"),
    ]


def test_records_keep_description_issue_and_found_launches(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    _mock_detail(http_mocker, 1201, _detail(1201, 7, "Login page times out"))

    output = read_stream(_CONNECTOR, _STREAM, config)

    record = output.records[0].record.data
    assert record["description"] == "The login form never answers on slow networks"
    assert record["issue"]["name"] == "EXP-7"
    assert record["foundAtLaunch"]["id"] == 100
    assert record["lastFoundLaunch"]["id"] == 101


def test_records_stamped_with_tenant_source_and_unique_key(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    _mock_detail(http_mocker, 1201, _detail(1201, 7, "Login page times out"))

    output = read_stream(_CONNECTOR, _STREAM, config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"
    assert record["unique_key"] == "test-tenant-test-source-1201"


def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    _mock_detail(http_mocker, 1201, _detail(1201, 7, "Login page times out"))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_defect_list_pagination_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)], last=False))
    http_mocker.get(
        api_request(DEFECTS_URL, paged({"projectId": "7"}, page_index=1)),
        page([_row(1202)], number=1),
    )
    _mock_detail(http_mocker, 1201, _detail(1201, 7, "Login page times out"))
    _mock_detail(http_mocker, 1202, _detail(1202, 7, "Report export hangs"))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [1201, 1202]


def test_defect_deleted_between_list_and_detail_is_skipped(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201), _row(1202)]))
    _mock_detail(http_mocker, 1201, error(404))
    _mock_detail(http_mocker, 1202, _detail(1202, 7, "Report export hangs"))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [1202]


def test_project_the_token_cannot_read_is_skipped(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    mock_defect_list(http_mocker, 12, error(403))
    _mock_detail(http_mocker, 1201, _detail(1201, 7, "Login page times out"))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [1201]


@pytest.mark.usefixtures("slept")
def test_server_error_on_detail_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    _mock_detail(http_mocker, 1201, [error(500), _detail(1201, 7, "Login page times out")])

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [1201]
