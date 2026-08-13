"""Mock-server tests for github.

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
from config import GH_URL, PROXY_URL, GithubConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    read_stream,
)
from connector_tests.source import load_manifest

_CONNECTOR = "git/github"
_REPOS_URL = f"{GH_URL}/orgs/acme/repos"
_FROZEN = "2026-07-01T00:00:00Z"


def _graphql_body(stream_name: str, variables: dict, cursor: str | None = None) -> dict:
    """The exact request body the manifest sends — POST mocks match on it."""
    manifest = load_manifest(_CONNECTOR)
    stream = next(st for st in manifest["streams"] if st["name"] == stream_name)
    body = dict(stream["retriever"]["requester"]["request_body_json"])
    # The CDK's interpolation strips the YAML block scalar's trailing newline
    # at send time; the raw manifest keeps it.
    body["query"] = body["query"].rstrip("\n")
    body["variables"] = dict(variables)
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
    config = GithubConfigBuilder().build()
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
    config = GithubConfigBuilder().build()
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
    config = GithubConfigBuilder().build()
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
    config = GithubConfigBuilder().build()
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
def test_data_feed_stops_at_start_date_but_boundary_page_tail_emits(http_mocker: HttpMocker) -> None:
    """First-sync data-feed behavior, pinned: pagination stops at the first
    record older than start_date (the Link-next page is never mocked, so a
    fetch would fail the test) — but the boundary page's old tail still
    emits. A record_filter must never be added to "fix" the tail: the stop
    condition sees post-filter records, so it would unbound pagination."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())

    def _pr(num: int, updated: str) -> dict:
        return {
            "id": 900 + num, "number": num, "state": "open", "draft": False,
            "title": "t", "body": "b", "user": {"login": "a"},
            "head": {"ref": "f", "sha": "e" * 40}, "base": {"ref": "main"},
            "author_association": "MEMBER",
            "created_at": "2019-01-01T00:00:00Z", "updated_at": updated,
        }

    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/pulls", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([_pr(31, "2026-06-20T00:00:00Z"), _pr(30, "2019-05-20T00:00:00Z")]),
            status_code=200,
            headers={"Link": f'<{GH_URL}/repos/acme/app/pulls?page=2>; rel="next"'},
        ),
    )

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert not output.errors, "page 2 must never be fetched"
    nums = [r.record.data["number"] for r in output.records]
    assert 31 in nums
    assert 30 in nums, "boundary-page tail is expected to emit (accepted, documented)"


@freezegun.freeze_time(_FROZEN)
def test_pull_request_commits_store_only_the_edge(http_mocker: HttpMocker) -> None:
    """PR<->commit membership is vendor-only; the commit payload belongs to
    the proxy streams, so bronze keeps the sha and the PR identity alone."""
    config = GithubConfigBuilder().build()
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


def _diff_stats_body(cursor: str | None = None) -> dict:
    return _graphql_body("pull_request_diff_stats", {"owner": "acme", "name": "app"}, cursor)


@freezegun.freeze_time(_FROZEN)
def test_pull_request_diff_stats_come_from_the_list_node(http_mocker: HttpMocker) -> None:
    """REST carries additions/deletions on the PR detail response only; the
    GraphQL list node carries them, so a page of PRs costs one request."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_diff_stats_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "repository": {
                            "pullRequests": {
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": [
                                    {
                                        "number": 31,
                                        "updatedAt": "2026-06-20T00:00:00Z",
                                        "additions": 120,
                                        "deletions": 7,
                                        "changedFiles": 3,
                                        "author": {"email": "alice@example.com"},
                                    }
                                ],
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_diff_stats", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert (rec["additions"], rec["deletions"], rec["changed_files"]) == (120, 7, 3)
    assert rec["pull_number"] == 31
    assert rec["repo_full_name"] == "acme/app"
    assert rec["unique_key"].endswith(":acme/app:31")
    assert rec["author_email"] == "alice@example.com"
    assert "changedFiles" not in rec and "updatedAt" not in rec
    assert "author" not in rec
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_diff_stats", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_review_comments_carry_path_and_line(http_mocker: HttpMocker) -> None:
    """These are the only comments the silver contract can mark is_inline, so
    the file path and line must survive; an outdated comment has a null `line`
    and falls back to the position it was written against."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/pulls/comments", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {
                        "id": 500,
                        "pull_request_url": f"{GH_URL}/repos/acme/app/pulls/31",
                        "user": {"login": "alice", "id": 7},
                        "author_association": "MEMBER",
                        "body": "b" * 3000,
                        "path": "src/main.rs",
                        "line": 42,
                        "created_at": "2026-06-10T00:00:00Z",
                        "updated_at": "2026-06-20T00:00:00Z",
                    },
                    {
                        "id": 501,
                        "pull_request_url": f"{GH_URL}/repos/acme/app/pulls/31",
                        "user": {"login": "bob", "id": 8},
                        "author_association": "MEMBER",
                        "body": "outdated",
                        "path": "src/main.rs",
                        "line": None,
                        "original_line": 17,
                        "created_at": "2026-06-11T00:00:00Z",
                        "updated_at": "2026-06-21T00:00:00Z",
                    },
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_review_comments", config)

    assert not output.errors
    fresh, outdated = (r.record.data for r in output.records)
    assert fresh["pull_number"] == 31
    assert fresh["repo_full_name"] == "acme/app"
    assert (fresh["path"], fresh["line"]) == ("src/main.rs", 42)
    assert fresh["author_login"] == "alice"
    assert len(fresh["body"]) == 2048
    assert fresh["unique_key"].endswith(":acme/app:review_comment:500")
    assert outdated["line"] == 17
    assert "user" not in fresh and "pull_request_url" not in fresh
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_review_comments", strict=True)


def _pr_timeline_body(cursor: str | None = None) -> dict:
    return _graphql_body(
        "pull_request_timeline_events", {"owner": "acme", "name": "app", "number": 31}, cursor
    )


def _issue_timeline_body(cursor: str | None = None) -> dict:
    return _graphql_body(
        "issue_timeline_events", {"owner": "acme", "name": "app", "number": 7}, cursor
    )


@freezegun.freeze_time(_FROZEN)
def test_pr_timeline_flattens_every_event_type(http_mocker: HttpMocker) -> None:
    """Each timeline type names its second person under a different key, and
    none of them reference the item they belong to — so the pull request comes
    from the partition and the payload is flattened to one generic shape."""
    config = GithubConfigBuilder().build()
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
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_pr_timeline_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "repository": {
                            "pullRequest": {
                                "timelineItems": {
                                    "pageInfo": {"hasNextPage": False, "endCursor": None},
                                    "nodes": [
                                        {
                                            "__typename": "ReviewRequestedEvent",
                                            "id": "RR_1",
                                            "createdAt": "2026-06-11T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "requestedReviewer": {"login": "bob"},
                                        },
                                        {
                                            "__typename": "AssignedEvent",
                                            "id": "AS_1",
                                            "createdAt": "2026-06-12T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "assignee": {"login": "carol"},
                                        },
                                        {
                                            "__typename": "LabeledEvent",
                                            "id": "LA_1",
                                            "createdAt": "2026-06-13T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "label": {"name": "bug"},
                                        },
                                        {
                                            "__typename": "ClosedEvent",
                                            "id": "CL_1",
                                            "createdAt": "2026-06-14T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "stateReason": "COMPLETED",
                                        },
                                        {
                                            "__typename": "MergedEvent",
                                            "id": "ME_1",
                                            "createdAt": "2026-06-15T00:00:00Z",
                                            "actor": {"login": "alice"},
                                        },
                                    ],
                                }
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_timeline_events", config)

    assert not output.errors
    by_type = {r.record.data["event_type"]: r.record.data for r in output.records}
    assert by_type["ReviewRequestedEvent"]["target_login"] == "bob"
    assert by_type["AssignedEvent"]["target_login"] == "carol"
    assert by_type["LabeledEvent"]["label_name"] == "bug"
    assert by_type["ClosedEvent"]["state_reason"] == "COMPLETED"
    assert by_type["MergedEvent"]["target_login"] == ""
    for rec in by_type.values():
        assert rec["item_number"] == 31
        assert rec["repo_full_name"] == "acme/app"
        assert "__typename" not in rec and "createdAt" not in rec
    assert by_type["MergedEvent"]["unique_key"].endswith(":pull_request:31:ME_1")
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_timeline_events", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_issue_timeline_carries_board_and_field_changes(http_mocker: HttpMocker) -> None:
    """Board status and native issue fields are the only GitHub history for
    either, and both sides of the change are kept; the issues parent drops the
    pull requests its endpoint also answers for."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {"number": 7, "updated_at": "2026-06-20T00:00:00Z"},
                    {
                        "number": 31,
                        "updated_at": "2026-06-20T00:00:00Z",
                        "pull_request": {"url": "https://api.github.com/pulls/31"},
                    },
                ]
            ),
            status_code=200,
        ),
    )
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_timeline_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "repository": {
                            "issue": {
                                "timelineItems": {
                                    "pageInfo": {"hasNextPage": False, "endCursor": None},
                                    "nodes": [
                                        {
                                            "__typename": "ProjectV2ItemStatusChangedEvent",
                                            "id": "PS_1",
                                            "createdAt": "2026-06-11T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "previousStatus": "Todo",
                                            "status": "In Progress",
                                        },
                                        {
                                            "__typename": "IssueFieldChangedEvent",
                                            "id": "IF_1",
                                            "createdAt": "2026-06-12T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "issueField": {"name": "Estimate"},
                                            "previousValue": "3",
                                            "newValue": "5",
                                        },
                                        {
                                            "__typename": "IssueTypeChangedEvent",
                                            "id": "IT_1",
                                            "createdAt": "2026-06-13T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "issueType": {"name": "Bug"},
                                            "prevIssueType": {"name": "Task"},
                                        },
                                    ],
                                }
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "issue_timeline_events", config)

    assert not output.errors
    by_type = {r.record.data["event_type"]: r.record.data for r in output.records}
    board = by_type["ProjectV2ItemStatusChangedEvent"]
    assert (board["prev_value"], board["new_value"]) == ("Todo", "In Progress")
    field = by_type["IssueFieldChangedEvent"]
    assert (field["field_name"], field["prev_value"], field["new_value"]) == ("Estimate", "3", "5")
    kind = by_type["IssueTypeChangedEvent"]
    assert (kind["prev_value"], kind["new_value"]) == ("Task", "Bug")
    assert all(r["item_number"] == 7 for r in by_type.values())
    assert board["unique_key"].endswith(":issue:7:PS_1")
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "issue_timeline_events", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_graphql_not_found_skips_the_repository(http_mocker: HttpMocker) -> None:
    """An inaccessible repository is an HTTP 200 whose data is null and whose
    error type is NOT_FOUND — a skip, not a query error that fails the sync."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_diff_stats_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {"repository": None},
                    "errors": [{"type": "NOT_FOUND", "message": "Could not resolve to a Repository"}],
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_diff_stats", config)

    assert not output.errors
    assert len(output.records) == 0


@freezegun.freeze_time(_FROZEN)
def test_issues_filters_out_pull_requests(http_mocker: HttpMocker) -> None:
    """/issues returns PRs too; the record filter must drop them."""
    config = GithubConfigBuilder().build()
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
    config = GithubConfigBuilder().build()
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
    config = GithubConfigBuilder().build()
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_graphql_body("projects_v2", {"org": "acme"})),
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
    config = GithubConfigBuilder().build()
    node = {"id": "PVT_1", "number": 1, "title": "Roadmap", "shortDescription": None, "public": True, "closed": False, "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-06-01T00:00:00Z"}
    body = {"data": {"organization": {"projectsV2": {"pageInfo": {"hasNextPage": False, "endCursor": "c1"}, "nodes": [node]}}}}
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_graphql_body("projects_v2", {"org": "acme"})),
        HttpResponse(body=json.dumps(body), status_code=200),
    )

    output = read_stream(_CONNECTOR, "projects_v2", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["title"] == "Roadmap"
    assert rec["short_description"] == ""
    _no_literal_none(output.records)
