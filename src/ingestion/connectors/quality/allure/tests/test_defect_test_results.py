from __future__ import annotations

from config import (
    AllureConfigBuilder,
    api_request,
    defect_test_results_url,
    error,
    mock_defect_list,
    mock_projects,
    page,
    paged,
)
from connector_tests import HttpMocker, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "defect_test_results"


def _row(defect_id: int) -> dict:
    return load_fixture(__file__, "defect_row.json", id=defect_id)


def _linked(result_id: int, status: str = "failed") -> dict:
    return load_fixture(__file__, "defect_test_result.json", id=result_id, status=status)


def _links_params(page_index: int | None = None) -> dict:
    return paged({}, page_index)


def test_records_carry_defect_and_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    http_mocker.get(api_request(defect_test_results_url(1201), _links_params()), page([_linked(5001)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    record = output.records[0].record.data
    assert record["id"] == 5001
    assert record["defect_id"] == 1201
    assert record["project_id"] == 7
    assert record["unique_key"] == "test-tenant-test-source-1201-5001"


def test_pagination_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    http_mocker.get(api_request(defect_test_results_url(1201), _links_params()), page([_linked(5001)], last=False))
    http_mocker.get(api_request(defect_test_results_url(1201), _links_params(1)), page([_linked(5002)], number=1))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [5001, 5002]


def test_defect_without_linked_results_emits_nothing(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    http_mocker.get(api_request(defect_test_results_url(1201), _links_params()), page([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert not output.records


def test_defect_deleted_mid_sync_is_skipped(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201), _row(1202)]))
    http_mocker.get(api_request(defect_test_results_url(1201), _links_params()), error(404))
    http_mocker.get(api_request(defect_test_results_url(1202), _links_params()), page([_linked(5002)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["defect_id"] for r in output.records] == [1202]


def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    mock_defect_list(http_mocker, 7, page([_row(1201)]))
    http_mocker.get(api_request(defect_test_results_url(1201), _links_params()), page([_linked(5001)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)
