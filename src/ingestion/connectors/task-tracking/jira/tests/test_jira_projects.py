"""Mock-server tests for the `jira_projects` stream.

Plain paginated stream: GET /rest/api/3/project/search, OffsetIncrement
paginator (page_size 50, maxResults/startAt request params), `values` extract
field, AddFields stamping tenant_id / source_id / unique_key + flattened
project fields.

Coverage matrix rows: full_refresh_single_page, pagination_multi_page,
empty_page, tenant_source_stamping, schema_conformance, error_retry (429),
error_ignore (400).
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    load_fixture,
    read_stream,
)

_STREAM = "jira_projects"
_CONNECTOR = "task-tracking/jira"
_SEARCH_URL = f"{JIRA_URL}/rest/api/3/project/search"


def _project(pid: int, key: str) -> dict:
    """The fixtures/project.json record with only the case-relevant overrides."""
    return load_fixture(
        __file__,
        "project.json",
        id=str(pid),
        key=key,
        name=f"Project {key}",
        self=f"{JIRA_URL}/rest/api/3/project/{pid}",
    )


def _page(records: list[dict], *, is_last: bool = True) -> HttpResponse:
    return HttpResponse(
        body=json.dumps({"values": records, "isLast": is_last}), status_code=200
    )


def test_full_refresh_single_page(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        _page([_project(10001, "PROJ1"), _project(10002, "PROJ2")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    keys = [r.record.data["key"] for r in output.records]
    assert keys == ["PROJ1", "PROJ2"]


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        _page([_project(10001, "PROJ1")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (
        f"{config['insight_tenant_id']}-{config['insight_source_id']}-10001"
    )
    # Flattening transformations declared in the manifest. CDK interpolation
    # literal-evals the rendered Jinja value, so the numeric-string API id
    # becomes an int — the schema accordingly declares project_id as number.
    assert rec["project_id"] == 10001
    assert rec["project_key"] == "PROJ1"
    assert rec["project_type"] == "software"


def test_schema_conformance(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        _page([_project(10001, "PROJ1"), _project(10002, "PROJ2")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_pagination_multi_page(http_mocker: HttpMocker) -> None:
    """OffsetIncrement: a full page (50 = page_size) triggers a second request
    with startAt=50; a short page stops pagination."""
    config = JiraConfigBuilder().build()
    page1 = [_project(10000 + i, f"P{i}") for i in range(50)]
    page2 = [_project(10100, "LAST")]

    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params={"maxResults": "50"}),
        _page(page1, is_last=False),
    )
    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params={"maxResults": "50", "startAt": "50"}),
        _page(page2),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 51
    assert output.records[-1].record.data["key"] == "LAST"


def test_empty_page(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params=ANY_QUERY_PARAMS), _page([])
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors


def test_error_retry_429(http_mocker: HttpMocker) -> None:
    """The manifest error handler RETRIES 429 (WaitTimeFromHeader Retry-After);
    the read must succeed once the source recovers, without losing records."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(
                body=json.dumps({"errorMessages": ["rate limited"]}),
                status_code=429,
                headers={"Retry-After": "0"},
            ),
            _page([_project(10001, "PROJ1")]),
        ],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    assert not output.errors


def test_error_ignore_400(http_mocker: HttpMocker) -> None:
    """The manifest error handler IGNOREs 400/404: no records, no failure."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps({"errorMessages": ["bad request"]}), status_code=400
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors
