from __future__ import annotations

import pytest
from config import AllureConfigBuilder, api_request, custom_fields_url, error, page, paged
from connector_tests import HttpMocker, assert_records_conform, load_fixture, read_stream

_CONNECTOR = "quality/allure"
_STREAM = "custom_fields"


def _field(field_id: int, name: str, project_id: int = 7) -> dict:
    return load_fixture(__file__, "custom_field.json", id=field_id, name=name, projectId=project_id)


def test_one_request_per_configured_project(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    first = api_request(custom_fields_url(7), paged({}))
    second = api_request(custom_fields_url(12), paged({}))
    http_mocker.get(first, page([_field(-2, "Feature"), _field(5, "SubProject")]))
    http_mocker.get(second, page([_field(-2, "Feature", project_id=12)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert sorted((r.record.data["projectId"], r.record.data["name"]) for r in output.records) == [
        (7, "Feature"),
        (7, "SubProject"),
        (12, "Feature"),
    ]
    http_mocker.assert_number_of_calls(first, 1)
    http_mocker.assert_number_of_calls(second, 1)


def test_unique_key_includes_project_because_global_fields_repeat(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().with_field("allure_project_ids", [7, 12]).build()
    http_mocker.get(api_request(custom_fields_url(7), paged({})), page([_field(-2, "Feature")]))
    http_mocker.get(api_request(custom_fields_url(12), paged({})), page([_field(-2, "Feature", project_id=12)]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert sorted(r.record.data["unique_key"] for r in output.records) == [
        "test-tenant-test-source-12--2",
        "test-tenant-test-source-7--2",
    ]


def test_records_stamped_with_tenant_and_source(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(custom_fields_url(7), paged({})), page([_field(5, "SubProject")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    record = output.records[0].record.data
    assert record["tenant_id"] == "test-tenant"
    assert record["source_id"] == "test-source"


def test_records_conform_to_schema(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(custom_fields_url(7), paged({})), page([_field(5, "SubProject")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_pagination_follows_page_index_until_last_page(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(custom_fields_url(7), paged({})), page([_field(-2, "Feature")], number=0, last=False))
    http_mocker.get(
        api_request(custom_fields_url(7), paged({}, page_index=1)), page([_field(5, "SubProject")], number=1, last=True)
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["name"] for r in output.records] == ["Feature", "SubProject"]


@pytest.mark.usefixtures("slept")
def test_server_error_is_retried(http_mocker: HttpMocker) -> None:
    config = AllureConfigBuilder().build()
    http_mocker.get(api_request(custom_fields_url(7), paged({})), [error(502), page([_field(5, "SubProject")])])

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["name"] for r in output.records] == ["SubProject"]
