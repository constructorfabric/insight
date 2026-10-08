from __future__ import annotations

import pytest
from config import AllureConfigBuilder, api_request, categories_url, error, mock_projects, page, paged
from connector_tests import HttpMocker, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "categories"


def _category(category_id: int, name: str) -> dict:
    return load_fixture(__file__, "category.json", id=category_id, name=name)


def test_one_request_per_configured_project_stamping_its_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_projects(http_mocker)
    first = api_request(categories_url(7), paged({}))
    second = api_request(categories_url(12), paged({}))
    http_mocker.get(first, page([_category(21, "Product defects"), _category(22, "Test defects")]))
    http_mocker.get(second, page([_category(21, "Product defects")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted((r.record.data["project_id"], r.record.data["name"]) for r in output.records) == [
        (7, "Product defects"),
        (7, "Test defects"),
        (12, "Product defects"),
    ]
    http_mocker.assert_number_of_calls(first, 1)
    http_mocker.assert_number_of_calls(second, 1)


def test_unique_key_includes_project_because_shared_categories_repeat(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(categories_url(7), paged({})), page([_category(21, "Product defects")]))
    http_mocker.get(api_request(categories_url(12), paged({})), page([_category(21, "Product defects")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert sorted(r.record.data["unique_key"] for r in output.records) == [
        "test-tenant-test-source-12-21",
        "test-tenant-test-source-7-21",
    ]


def test_records_stamped_with_tenant_and_source(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(categories_url(7), paged({})), page([_category(21, "Product defects")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"


def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(categories_url(7), paged({})), page([_category(21, "Product defects")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_pagination_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(categories_url(7), paged({})), page([_category(21, "Product defects")], last=False))
    http_mocker.get(
        api_request(categories_url(7), paged({}, page_index=1)), page([_category(22, "Test defects")], number=1)
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["name"] for r in output.records] == ["Product defects", "Test defects"]


def test_project_the_token_cannot_read_is_skipped(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(categories_url(7), paged({})), page([_category(21, "Product defects")]))
    http_mocker.get(api_request(categories_url(12), paged({})), error(403))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["project_id"] for r in output.records] == [7]


@pytest.mark.usefixtures("slept")
def test_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(categories_url(7), paged({})), [error(502), page([_category(21, "Product defects")])])

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["name"] for r in output.records] == ["Product defects"]
