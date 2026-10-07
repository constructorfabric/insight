from __future__ import annotations

from config import AllureConfigBuilder, api_request, category_matchers_url, error, mock_projects, page, paged
from connector_tests import HttpMocker, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "category_matchers"


def _matcher(matcher_id: int, name: str) -> dict:
    return load_fixture(__file__, "category_matcher.json", id=matcher_id, name=name)


def test_one_request_per_configured_project_stamping_its_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(category_matchers_url(7), paged({})), page([_matcher(91, "Element not found")]))
    http_mocker.get(api_request(category_matchers_url(12), paged({})), page([_matcher(91, "Element not found")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["project_id"] for r in output.records) == [7, 12]
    assert sorted(r.record.data["unique_key"] for r in output.records) == [
        "test-tenant-test-source-12-91",
        "test-tenant-test-source-7-91",
    ]


def test_records_keep_the_category_and_both_regexes(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(category_matchers_url(7), paged({})), page([_matcher(91, "Element not found")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    record = output.records[0].record.data
    assert record["category"]["id"] == 22
    assert record["messageRegex"] == ".*NoSuchElementException.*"
    assert record.get("traceRegex") is None


def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(category_matchers_url(7), paged({})), page([_matcher(91, "Element not found")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_pagination_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(category_matchers_url(7), paged({})), page([_matcher(91, "First")], last=False))
    http_mocker.get(api_request(category_matchers_url(7), paged({}, page_index=1)), page([_matcher(92, "Second")], number=1))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["name"] for r in output.records] == ["First", "Second"]


def test_project_the_token_cannot_read_is_skipped(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    mock_projects(http_mocker)
    http_mocker.get(api_request(category_matchers_url(7), paged({})), page([_matcher(91, "Element not found")]))
    http_mocker.get(api_request(category_matchers_url(12), paged({})), error(403))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["project_id"] for r in output.records] == [7]
