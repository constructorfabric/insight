from __future__ import annotations

import logging

import pytest
from airbyte_cdk.models import Status
from config import ALLURE_URL, PROJECTS_URL, AllureConfigBuilder, api_request, error, page, paged
from connector_tests import HttpMocker, assert_records_conform, get_source, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "projects"


def _project(project_id: int, name: str) -> dict:
    return load_fixture(__file__, "project.json", id=project_id, name=name)


def test_full_refresh_single_page_emits_every_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), page([_project(7, "Alpha"), _project(12, "Beta")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted(r.record.data["id"] for r in output.records) == [7, 12]


def test_records_stamped_with_tenant_source_and_unique_key(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), page([_project(7, "Alpha")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"
    assert record["unique_key"] == "test-tenant-test-source-7"


def test_records_carry_the_instance_url(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), page([_project(7, "Alpha")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.records[0].record.data["allure_url"] == ALLURE_URL


def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), page([_project(7, "Alpha")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_empty_page_emits_nothing(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), page([]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert not output.records


def test_pagination_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), page([_project(7, "Alpha")], number=0, last=False))
    http_mocker.get(
        api_request(PROJECTS_URL, paged({}, page_index=1)), page([_project(12, "Beta")], number=1, last=True)
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [7, 12]


def test_every_request_authenticates_with_the_api_token_header(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    first = api_request(PROJECTS_URL, paged({}))
    second = api_request(PROJECTS_URL, paged({}, page_index=1))
    http_mocker.get(first, page([_project(7, "Alpha")], number=0, last=False))
    http_mocker.get(second, page([_project(12, "Beta")], number=1, last=True))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    http_mocker.assert_number_of_calls(first, 1)
    http_mocker.assert_number_of_calls(second, 1)


def test_configured_page_size_is_requested(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_page_size", "250").build()
    http_mocker.get(api_request(PROJECTS_URL, paged({}, size=250)), page([_project(7, "Alpha")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 1


@pytest.mark.parametrize("page_size", ["0", "1001", "ten"])
def test_page_size_outside_one_to_thousand_is_rejected(page_size: str) -> None:
    config = AllureConfigBuilder().with_field("allure_page_size", page_size).build()

    with pytest.raises(ValueError, match="does not match"):
        get_source(_CONNECTOR, config)


def test_rate_limited_request_waits_for_retry_after_then_succeeds(http_mocker: HttpMocker, slept: list) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(
        api_request(PROJECTS_URL, paged({})), [error(429, {"Retry-After": "30"}), page([_project(7, "Alpha")])]
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [7]
    assert len(slept) == 1
    assert 30 <= slept[0] <= 31


@pytest.mark.usefixtures("slept")
@pytest.mark.parametrize("status", [500, 502, 503, 504])
def test_server_error_is_retried_then_succeeds(http_mocker: HttpMocker, status: int) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), [error(status), page([_project(7, "Alpha")])])

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [7]


@pytest.mark.usefixtures("slept")
def test_server_error_gives_up_after_five_retries(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    request = api_request(PROJECTS_URL, paged({}))
    http_mocker.get(request, error(503))

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.errors
    assert not output.records
    http_mocker.assert_number_of_calls(request, 6)


def test_check_reads_projects(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(PROJECTS_URL, paged({})), page([_project(7, "Alpha")]))

    status = get_source(_CONNECTOR, config).check(logging.getLogger("airbyte"), config)

    assert status.status == Status.SUCCEEDED


def test_allure_url_with_trailing_slash_is_rejected() -> None:
    config = AllureConfigBuilder().with_field("allure_url", "https://allure.example.test/").build()

    with pytest.raises(ValueError, match="does not match"):
        get_source(_CONNECTOR, config)


def test_plain_http_allure_url_is_rejected() -> None:
    config = AllureConfigBuilder().with_field("allure_url", "http://allure.example.test").build()

    with pytest.raises(ValueError, match="does not match"):
        get_source(_CONNECTOR, config)
