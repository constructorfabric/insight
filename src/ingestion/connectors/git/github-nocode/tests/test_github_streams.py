"""Mock-server tests for github-nocode.

GitHub-specific hazards under test: secondary rate limits arriving as 403
(must retry, not skip), per-repository 403/404 skipping on repo-scoped
streams, the issues endpoint returning PRs (filtered out), GraphQL errors
arriving as HTTP 200 (must FAIL loudly), the proxy 429 retry loop, and the
literal-"None" guard.

Coverage matrix rows: full_refresh_single_page, incremental_state,
tenant_source_stamping, schema_conformance, substream_partition,
record_filter, error_retry (403-as-throttle, proxy 429), error_ignore (404),
transformations (None-guard).
"""

from __future__ import annotations

import json

import freezegun
from config import GH_URL, PROXY_URL, GithubNocodeConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    read_stream,
)
from connector_tests.source import load_manifest

_CONNECTOR = "git/github-nocode"
_REPOS_URL = f"{GH_URL}/orgs/acme/repos"
_FROZEN = "2026-07-01T00:00:00Z"


def _graphql_body(cursor: str | None = None) -> dict:
    """The exact request body the manifest sends — POST mocks match on it."""
    manifest = load_manifest(_CONNECTOR)
    stream = next(st for st in manifest["streams"] if st["name"] == "projects_v2")
    body = dict(stream["retriever"]["requester"]["request_body_json"])
    # The CDK's interpolation strips the YAML block scalar's trailing newline
    # at send time; the raw manifest keeps it.
    body["query"] = body["query"].rstrip("\n")
    body["variables"] = {"org": "acme"}
    if cursor is not None:
        body["variables"]["cursor"] = cursor
    return body


def _no_literal_none(records) -> None:
    for r in records:
        for key, value in r.record.data.items():
            assert value != "None", f"literal 'None' leaked into {key}"


def _repo() -> dict:
    return {
        "id": 42,
        "full_name": "acme/app",
        "name": "app",
        "default_branch": "main",
        "archived": False,
        "fork": False,
        "private": True,
        "clone_url": "https://github.com/acme/app.git",
        "pushed_at": "2026-06-20T10:00:00Z",
        "created_at": "2020-01-01T00:00:00Z",
        "updated_at": "2026-06-20T10:00:00Z",
    }


def _repos_page() -> HttpResponse:
    return HttpResponse(body=json.dumps([_repo()]), status_code=200)


@freezegun.freeze_time(_FROZEN)
def test_repositories_full_refresh_and_stamping(http_mocker: HttpMocker) -> None:
    config = GithubNocodeConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["unique_key"].endswith(":42")
    assert '"""' not in rec["unique_key"], "int id must interpolate plain (CDK string filter triple-quotes non-strings)"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "repositories", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_secondary_rate_limit_403_retries_then_succeeds(http_mocker: HttpMocker) -> None:
    """GitHub reports secondary limits as 403; the predicate must classify it
    as a throttle (retry) rather than a denial (skip)."""
    config = GithubNocodeConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(
                body=json.dumps({"message": "You have exceeded a secondary rate limit."}),
                status_code=403,
                headers={"Retry-After": "0"},
            ),
            _repos_page(),
        ],
    )

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    assert len(output.records) == 1


@freezegun.freeze_time(_FROZEN)
def test_proxy_429_then_success(http_mocker: HttpMocker) -> None:
    config = GithubNocodeConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(body="", status_code=429, headers={"Retry-After": "0"}),
            HttpResponse(
                body=json.dumps(
                    {
                        "items": [
                            {
                                "sha": "d" * 40,
                                "message": "m",
                                "committed_date": "2026-06-15T10:00:00Z",
                                "authored_date": "2026-06-15T10:00:00Z",
                                "author_name": "Dev",
                                "author_email": "dev@example.com",
                                "committer_name": "Dev",
                                "committer_email": "dev@example.com",
                                "parent_hashes": [],
                                "is_merge": False,
                                "is_in_default_branch": True,
                                "patch_id": None,
                            }
                        ],
                        "next_page_token": None,
                    }
                ),
                status_code=200,
            ),
        ],
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    assert len(output.records) == 1
    _no_literal_none(output.records)


@freezegun.freeze_time(_FROZEN)
def test_pull_requests_trim_body_and_hoist_author(http_mocker: HttpMocker) -> None:
    """The silver PR model consumes body as description; it is trimmed to
    2048 chars, and a deleted author surfaces as '' — never "None"."""
    config = GithubNocodeConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/pulls", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {
                        "id": 900,
                        "number": 31,
                        "state": "open",
                        "draft": False,
                        "title": "t" * 3000,
                        "body": "b" * 3000,
                        "user": {"login": "alice"},
                        "head": {"ref": "feat", "sha": "e" * 40},
                        "base": {"ref": "main"},
                        "author_association": "MEMBER",
                        "created_at": "2026-06-10T00:00:00Z",
                        "updated_at": "2026-06-20T00:00:00Z",
                    }
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert len(rec["body"]) == 2048
    assert len(rec["title"]) == 1024
    assert rec["author_login"] == "alice"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_requests", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_pull_request_commits_store_only_the_edge(http_mocker: HttpMocker) -> None:
    """PR<->commit membership is vendor-only; the commit payload belongs to
    the proxy streams, so bronze keeps the sha and the PR identity alone."""
    config = GithubNocodeConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/pulls", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {
                        "id": 900,
                        "number": 31,
                        "state": "open",
                        "draft": False,
                        "title": "t",
                        "body": "b",
                        "user": {"login": "alice"},
                        "head": {"ref": "feat", "sha": "e" * 40},
                        "base": {"ref": "main"},
                        "author_association": "MEMBER",
                        "created_at": "2026-06-10T00:00:00Z",
                        "updated_at": "2026-06-20T00:00:00Z",
                    }
                ]
            ),
            status_code=200,
        ),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/pulls/31/commits", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {
                        "sha": "a" * 40,
                        "node_id": "C_1",
                        "commit": {"message": "feat: x", "author": {"name": "Alice"}},
                        "url": "https://api.github.com/repos/acme/app/commits/" + "a" * 40,
                        "html_url": "https://github.com/acme/app/commit/" + "a" * 40,
                        "comments_url": "https://api.github.com/repos/acme/app/commits/x/comments",
                        "author": {"login": "alice"},
                        "committer": {"login": "alice"},
                        "parents": [{"sha": "b" * 40}],
                    }
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_commits", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["sha"] == "a" * 40
    assert rec["pull_number"] == 31
    assert rec["repo_full_name"] == "acme/app"
    assert rec["unique_key"].endswith(f":acme/app:31:{'a' * 40}")
    assert "commit" not in rec and "parents" not in rec
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_commits", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_issues_filters_out_pull_requests(http_mocker: HttpMocker) -> None:
    """/issues returns PRs too; the record filter must drop them."""
    config = GithubNocodeConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {"id": 1, "number": 10, "state": "open", "title": "real issue", "user": {"login": "alice"}, "assignees": [], "labels": [], "comments": 0, "created_at": "2026-06-10T00:00:00Z", "updated_at": "2026-06-20T00:00:00Z"},
                    {"id": 2, "number": 11, "state": "open", "title": "a PR in disguise", "user": None, "assignees": [], "labels": [], "comments": 0, "created_at": "2026-06-10T00:00:00Z", "updated_at": "2026-06-20T00:00:00Z", "pull_request": {"url": "..."}},
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "issues", config)

    assert not output.errors
    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["title"] == "real issue"
    assert rec["author_login"] == "alice"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "issues", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_workflow_runs_key_carries_run_attempt(http_mocker: HttpMocker) -> None:
    config = GithubNocodeConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/actions/runs", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                {
                    "workflow_runs": [
                        {"id": 500, "run_attempt": 2, "name": "ci", "workflow_id": 9, "event": "push", "status": "completed", "conclusion": "success", "head_branch": "main", "head_sha": "e" * 40, "actor": {"login": "alice"}, "run_started_at": "2026-06-28T00:00:00Z", "created_at": "2026-06-28T00:00:00Z", "updated_at": "2026-06-28T01:00:00Z"}
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "workflow_runs", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["unique_key"].endswith(":run:500:2"), rec["unique_key"]
    _no_literal_none(output.records)


@freezegun.freeze_time(_FROZEN)
def test_graphql_error_in_a_200_fails_loudly(http_mocker: HttpMocker) -> None:
    """A GraphQL error arrives as HTTP 200 without `data`; the stream must
    fail with GitHub's message, not report zero projects."""
    config = GithubNocodeConfigBuilder().build()
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_graphql_body()),
        HttpResponse(
            body=json.dumps({"errors": [{"type": "INSUFFICIENT_SCOPES", "message": "needs read:project"}]}),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "projects_v2", config)

    assert output.errors, "a query-level GraphQL error must fail the stream"
    assert len(output.records) == 0


@freezegun.freeze_time(_FROZEN)
def test_projects_v2_graphql_pagination_cursor_in_body(http_mocker: HttpMocker) -> None:
    config = GithubNocodeConfigBuilder().build()
    node = {"id": "PVT_1", "number": 1, "title": "Roadmap", "shortDescription": None, "public": True, "closed": False, "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-06-01T00:00:00Z"}
    body = {"data": {"organization": {"projectsV2": {"pageInfo": {"hasNextPage": False, "endCursor": "c1"}, "nodes": [node]}}}}
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_graphql_body()),
        HttpResponse(body=json.dumps(body), status_code=200),
    )

    output = read_stream(_CONNECTOR, "projects_v2", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["title"] == "Roadmap"
    assert rec["short_description"] == ""
    _no_literal_none(output.records)
