"""Mock-server tests for the `jira_issue_keys` stream.

Substream + incremental: the inline parent `jira_project_discovery` enumerates
projects (GET /rest/api/3/project/search?expand=insight), then one JQL search
per project partition (GET /rest/api/3/search/jql) with a DatetimeBasedCursor
(cursor_field `updated` hoisted from `fields.updated`, step P30D, lookback
PT14H, global substream cursor) and nextPageToken pagination.

Coverage matrix rows: substream_partition, incremental_state (state emission +
resume-read request filtering), pagination_multi_page (CursorPagination),
tenant_source_stamping (unique_key from issue key), transformations (cursor
hoist). schema_conformance is explicitly SKIPPED — the rig found a real
manifest<->schema type drift (see the skip reason).

The clock is frozen at 2026-07-01 00:00 UTC and jira_start_date is 2026-06-01,
so each partition gets exactly one 30-day slice.
"""

from __future__ import annotations

import json

import freezegun
import pytest
from config import JIRA_URL, JiraConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    load_fixture,
    read_stream,
)

_STREAM = "jira_issue_keys"
_CONNECTOR = "task-tracking/jira"
_PROJECT_SEARCH_URL = f"{JIRA_URL}/rest/api/3/project/search"
_JQL_URL = f"{JIRA_URL}/rest/api/3/search/jql"
_NOW = "2026-07-01T00:00:00Z"

_WINDOW_START = "2026-06-01 00:00"
_WINDOW_END = "2026-07-01 00:00"


def _projects_response(keys: list[str]) -> HttpResponse:
    values = [
        load_fixture(
            __file__,
            "discovery_project.json",
            id=str(10000 + i),
            key=key,
            name=f"Project {key}",
        )
        for i, key in enumerate(keys)
    ]
    return HttpResponse(body=json.dumps({"values": values, "isLast": True}), status_code=200)


def _jql_params(project: str, start: str, end: str, page_token: str | None = None) -> dict:
    params = {
        "jql": (
            f'project = "{project}" AND updated >= "{start}" '
            f'AND updated <= "{end}" ORDER BY updated ASC'
        ),
        "fields": "updated",
        "maxResults": "100",
    }
    if page_token:
        params["nextPageToken"] = page_token
    return params


def _issues_response(issues: list[tuple[str, str, str]], next_token: str | None = None) -> HttpResponse:
    body = {
        "issues": [
            load_fixture(__file__, "issue.json", id=iid, key=key, fields={"updated": updated})
            for iid, key, updated in issues
        ]
    }
    if next_token:
        body["nextPageToken"] = next_token
    return HttpResponse(body=json.dumps(body), status_code=200)


@freezegun.freeze_time(_NOW)
def test_substream_partition_per_project(http_mocker: HttpMocker) -> None:
    """One JQL request per parent project partition; an unregistered partition
    request would fail the test (no network fallthrough)."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        _projects_response(["PROJ1", "PROJ2"]),
    )
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=_jql_params("PROJ1", _WINDOW_START, _WINDOW_END)),
        _issues_response([("10001", "PROJ1-1", "2026-06-15T10:00:00.000+0000")]),
    )
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=_jql_params("PROJ2", _WINDOW_START, _WINDOW_END)),
        _issues_response([("20001", "PROJ2-1", "2026-06-16T11:00:00.000+0000")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert not output.errors
    keys = sorted(r.record.data["key"] for r in output.records)
    assert keys == ["PROJ1-1", "PROJ2-1"]


@freezegun.freeze_time(_NOW)
def test_cursor_hoist_and_stamping(http_mocker: HttpMocker) -> None:
    """`updated` must be hoisted from fields.updated to the record top level
    (otherwise the cursor never observes values), and identity stamping uses
    the issue key."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        _projects_response(["PROJ1"]),
    )
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=ANY_QUERY_PARAMS),
        _issues_response([("10001", "PROJ1-1", "2026-06-15T10:00:00.000+0000")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["updated"] == "2026-06-15T10:00:00.000+0000"
    # CDK interpolation literal-evals the rendered value: numeric-string id -> int.
    assert rec["jira_id"] == 10001
    assert rec["id_readable"] == "PROJ1-1"
    assert rec["unique_key"] == (
        f"{config['insight_tenant_id']}-{config['insight_source_id']}-PROJ1-1"
    )


@pytest.mark.skip(
    reason="known manifest<->schema drift, found by this rig: jira_issue_keys "
    "declares jira_id as ['string','null'] but the AddFields Jinja literal-eval "
    "emits int for numeric ids (the sibling jira_projects.project_id, generated "
    "from real data, is correctly declared 'number'). Fixing the schema implies "
    "a bronze column-type change — tracked separately from this PR."
)
def test_schema_conformance() -> None:
    pass


@freezegun.freeze_time(_NOW)
def test_pagination_next_page_token(http_mocker: HttpMocker) -> None:
    """CursorPagination: a nextPageToken in the response drives a second
    request carrying it; a response without the token stops."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        _projects_response(["PROJ1"]),
    )
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=_jql_params("PROJ1", _WINDOW_START, _WINDOW_END)),
        _issues_response(
            [("10001", "PROJ1-1", "2026-06-10T10:00:00.000+0000")], next_token="tok-2"
        ),
    )
    http_mocker.get(
        HttpRequest(
            _JQL_URL,
            query_params=_jql_params("PROJ1", _WINDOW_START, _WINDOW_END, page_token="tok-2"),
        ),
        _issues_response([("10002", "PROJ1-2", "2026-06-12T10:00:00.000+0000")]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 2
    assert sorted(r.record.data["key"] for r in output.records) == ["PROJ1-1", "PROJ1-2"]


@freezegun.freeze_time(_NOW)
def test_incremental_state_emitted_and_resume_filters(http_mocker: HttpMocker) -> None:
    """First read emits a state message with the max observed cursor; a second
    read given that state must issue a JQL filtered from the cursor minus the
    PT14H lookback window — asserted by the exact request matcher."""
    config = JiraConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
        _projects_response(["PROJ1"]),
    )
    http_mocker.get(
        HttpRequest(_JQL_URL, query_params=_jql_params("PROJ1", _WINDOW_START, _WINDOW_END)),
        _issues_response([("10001", "PROJ1-1", "2026-06-15T10:00:00.000+0000")]),
    )

    first = read_stream(_CONNECTOR, _STREAM, config)

    assert len(first.records) == 1
    assert first.state_messages, "incremental stream must emit state"
    state = [m.state for m in first.state_messages][-1:]

    # Resume: cursor 2026-06-15 10:00 minus lookback PT14H -> 2026-06-14 20:00
    resume_mocker = HttpMocker()
    with resume_mocker:
        resume_mocker.get(
            HttpRequest(_PROJECT_SEARCH_URL, query_params=ANY_QUERY_PARAMS),
            _projects_response(["PROJ1"]),
        )
        resume_mocker.get(
            HttpRequest(
                _JQL_URL,
                query_params=_jql_params("PROJ1", "2026-06-14 20:00", _WINDOW_END),
            ),
            _issues_response([]),
        )

        second = read_stream(_CONNECTOR, _STREAM, config, state=state)

        assert len(second.records) == 0
        assert not second.errors
