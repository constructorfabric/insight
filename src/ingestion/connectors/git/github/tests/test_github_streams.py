"""Mock-server tests for github.

GitHub-specific hazards under test: secondary rate limits arriving as 403
(must retry, not skip), per-repository 403/404 skipping on repo-scoped
streams, the issues endpoint returning PRs (filtered out), GraphQL errors
arriving as HTTP 200 (a rate limit must retry, a query error must FAIL
loudly), the proxy 429 retry loop, and the literal-"None" guard.

Coverage matrix rows: full_refresh_single_page, incremental_state,
tenant_source_stamping, schema_conformance, substream_partition,
record_filter, error_retry (403-as-throttle, GraphQL rate limit in a 200,
proxy 429), error_ignore (404),
transformations (None-guard).
"""

from __future__ import annotations

import json
from datetime import datetime
from urllib.parse import parse_qs, urlparse

import freezegun
import pytest
from airbyte_cdk.models import FailureType
from config import GH_URL, PROXY_URL, GithubConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, assert_records_conform, read_stream
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


def _instant(stamp: str) -> datetime:
    return datetime.strptime(stamp.replace("Z", "+00:00"), "%Y-%m-%dT%H:%M:%S%z")


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
        "size": 716800,
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
def test_repository_roster_admits_neither_forks_nor_archived(http_mocker: HttpMocker) -> None:
    """A fork's clone carries its whole upstream history, so admitting one
    credits every upstream commit to this organization; an archived repository
    is not active work. Both stay out of the roster every other stream
    partitions over."""
    config = GithubConfigBuilder().build()
    fork = _repo() | {"id": 43, "full_name": "acme/forked", "name": "forked", "fork": True}
    archived = _repo() | {"id": 44, "full_name": "acme/old", "name": "old", "archived": True}
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_repo(), fork, archived]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    assert [r.record.data["full_name"] for r in output.records] == ["acme/app"]


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
                # A real secondary-limit 403 still reports hourly budget left.
                # Without X-RateLimit-Remaining the api_budget layer reads a
                # ratelimit-hit status as budget 0 and, with no reset header,
                # sleeps out the rest of the fixed one-hour window — which
                # hangs this test for an hour instead of retrying instantly.
                headers={"Retry-After": "0", "X-RateLimit-Remaining": "4999"},
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
            _commits_page(_commit_row("d" * 40, "2026-06-15T10:00:00Z")),
        ],
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    hints = {r.headers.get("X-Repo-Size-Hint") for r in http_mocker._mocker.request_history if "/v1/commits" in r.url}
    assert hints == {"734003200"}, (
        f"every proxy call carries the repository's size in bytes (GitHub reports KiB): {hints}"
    )
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
                        "user": {"login": "alice", "id": 4242, "type": "User"},
                        "head": {
                            "ref": "feat",
                            "sha": "e" * 40,
                            "label": "contributor:feat",
                            "repo": {"full_name": "contributor/app"},
                        },
                        "base": {"ref": "main", "sha": "f" * 40},
                        "author_association": "CONTRIBUTOR",
                        "labels": [{"name": "bug"}, {"name": "urgent"}],
                        "assignees": [{"login": "bob"}],
                        "requested_reviewers": [{"login": "carol"}],
                        "requested_teams": [{"slug": "platform"}],
                        "milestone": {"title": "v2", "number": 9},
                        "auto_merge": None,
                        "locked": False,
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
    assert (rec["author_id"], rec["author_type"]) == (4242, "User")
    # A head branch in another repository is the only fork signal here.
    assert rec["head_repo_full_name"] == "contributor/app"
    assert rec["head_label"] == "contributor:feat"
    assert rec["base_sha"] == "f" * 40
    assert json.loads(rec["label_names"]) == ["bug", "urgent"]
    assert json.loads(rec["assignee_logins"]) == ["bob"]
    assert json.loads(rec["requested_reviewer_logins"]) == ["carol"]
    assert json.loads(rec["requested_team_slugs"]) == ["platform"]
    assert (rec["milestone_title"], rec["milestone_number"]) == ("v2", 9)
    assert rec["auto_merge_enabled"] is False
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_requests", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_data_feed_stops_at_start_date_and_drops_the_boundary_page_tail(http_mocker: HttpMocker) -> None:
    """First-sync data-feed behavior, pinned: pagination stops at the first
    record older than start_date (the Link-next page is never mocked, so a
    fetch would fail the test), and the boundary page's older tail is
    dropped by the cursor's own window check. No record_filter is needed
    for that, and none must be added: the stop condition sees post-filter
    records, so a filter would unbound pagination."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())

    def _pr(num: int, updated: str) -> dict:
        return {
            "id": 900 + num,
            "number": num,
            "state": "open",
            "draft": False,
            "title": "t",
            "body": "b",
            "user": {"login": "a"},
            "head": {"ref": "f", "sha": "e" * 40},
            "base": {"ref": "main"},
            "author_association": "MEMBER",
            "created_at": "2019-01-01T00:00:00Z",
            "updated_at": updated,
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
    assert nums == [31], f"the in-window record emits, the older tail does not: {nums}"


@freezegun.freeze_time(_FROZEN)
def test_pull_request_commits_carry_the_commit_email_and_its_account(http_mocker: HttpMocker) -> None:
    """A commit's e-mail and the GitHub account it belongs to appear together
    nowhere else, so both survive alongside the membership edge and the commit's
    own metadata — the proxy cannot see a fork's head commits at all."""
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
                        "commit": {
                            "message": "feat: x",
                            "author": {"name": "Alice", "email": "alice@example.com", "date": "2026-06-11T09:00:00Z"},
                            "committer": {
                                "name": "Alice",
                                "email": "1234+alice@users.noreply.github.com",
                                "date": "2026-06-11T09:05:00Z",
                            },
                            "verification": {"verified": True, "reason": "valid"},
                        },
                        "url": "https://api.github.com/repos/acme/app/commits/" + "a" * 40,
                        "html_url": "https://github.com/acme/app/commit/" + "a" * 40,
                        "comments_url": "https://api.github.com/repos/acme/app/commits/x/comments",
                        "author": {"login": "alice", "id": 4242, "type": "User"},
                        "committer": {"login": "alice", "id": 4242, "type": "User"},
                        "parents": [{"sha": "b" * 40}, {"sha": "c" * 40}],
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
    assert rec["author_login"] == "alice"
    assert rec["author_id"] == 4242
    assert rec["author_type"] == "User"
    assert rec["author_email"] == "alice@example.com"
    assert rec["author_name"] == "Alice"
    assert rec["authored_date"] == "2026-06-11T09:00:00Z"
    assert rec["committer_login"] == "alice"
    assert rec["committer_email"] == "1234+alice@users.noreply.github.com"
    assert rec["committed_date"] == "2026-06-11T09:05:00Z"
    assert rec["message"] == "feat: x"
    assert rec["is_verified"] is True
    assert rec["verification_reason"] == "valid"
    assert json.loads(rec["parent_shas"]) == ["b" * 40, "c" * 40]
    assert rec["is_merge"] is True
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
                                        "author": {"login": "alice", "databaseId": 4242, "email": "alice@example.com"},
                                        "mergedBy": {"login": "bob", "databaseId": 77},
                                        "reviewDecision": "APPROVED",
                                        "totalCommentsCount": 5,
                                        "isCrossRepository": False,
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
    # REST reports merged_at but never who merged it.
    assert (rec["merged_by_login"], rec["merged_by_id"]) == ("bob", 77)
    assert (rec["author_login"], rec["author_id"]) == ("alice", 4242)
    assert rec["review_decision"] == "APPROVED"
    assert rec["total_comments_count"] == 5
    assert rec["is_cross_repository"] is False
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
    return _graphql_body("pull_request_timeline_events", {"owner": "acme", "name": "app", "number": 31}, cursor)


def _issue_timeline_body(cursor: str | None = None) -> dict:
    return _graphql_body("issue_timeline_events", {"owner": "acme", "name": "app", "number": 7}, cursor)


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
                                            "__typename": "ConnectedEvent",
                                            "id": "CE_1",
                                            "createdAt": "2026-06-14T00:00:00Z",
                                            "actor": {"login": "alice", "databaseId": 1001},
                                            "isCrossRepository": True,
                                            "subject": {
                                                "__typename": "Issue",
                                                "number": 7,
                                                "repository": {"nameWithOwner": "acme/other"},
                                            },
                                        },
                                        {
                                            "__typename": "DisconnectedEvent",
                                            "id": "DE_1",
                                            "createdAt": "2026-06-15T00:00:00Z",
                                            "actor": {"login": "alice", "databaseId": 1001},
                                            "isCrossRepository": False,
                                            "subject": {
                                                "__typename": "Issue",
                                                "number": 7,
                                                "repository": {"nameWithOwner": "acme/other"},
                                            },
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
    # A link made from the pull-request side is an event on the PULL REQUEST and
    # exists nowhere else, so the pair has to be requested here as well as on the
    # issue timeline — and as a PAIR, or an interval opened here never closes.
    for event_type in ("ConnectedEvent", "DisconnectedEvent"):
        assert by_type[event_type]["link_target_number"] == 7, event_type
        assert by_type[event_type]["link_target_repo_full_name"] == "acme/other", event_type
        assert by_type[event_type]["link_target_type"] == "Issue", event_type
    assert by_type["ConnectedEvent"]["is_cross_repository"] is True
    assert by_type["DisconnectedEvent"]["is_cross_repository"] is False
    assert by_type["MergedEvent"]["link_target_number"] == 0, "a non-link event carries no target"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_timeline_events", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_issue_timeline_carries_board_and_field_changes(http_mocker: HttpMocker) -> None:
    """Board status and native issue fields are the only GitHub history for
    either, and both sides of the change are kept; the issues parent drops the
    pull requests its endpoint also answers for."""
    config = GithubConfigBuilder().build()
    _mock_no_boards(http_mocker)
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
                                            "issueField": {"id": "IFN_1", "name": "Estimate"},
                                            "previousValue": "3",
                                            "newValue": "5",
                                        },
                                        {
                                            "__typename": "IssueTypeChangedEvent",
                                            "id": "IT_1",
                                            "createdAt": "2026-06-13T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "issueType": {"id": "IT_bug", "name": "Bug"},
                                            "prevIssueType": {"id": "IT_task", "name": "Task"},
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
    # Identifiers, not just labels: a rename must not orphan the history.
    assert field["field_id"] == "IFN_1"
    assert (kind["prev_value_id"], kind["new_value_id"]) == ("IT_task", "IT_bug")
    assert board["field_id"] == "", "a board move changes no native issue field"
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
                    {
                        "id": 1,
                        "number": 10,
                        "state": "open",
                        "title": "real issue",
                        "user": {"login": "alice"},
                        "assignees": [],
                        "labels": [],
                        "comments": 0,
                        "created_at": "2026-06-10T00:00:00Z",
                        "updated_at": "2026-06-20T00:00:00Z",
                    },
                    {
                        "id": 2,
                        "number": 11,
                        "state": "open",
                        "title": "a PR in disguise",
                        "user": None,
                        "assignees": [],
                        "labels": [],
                        "comments": 0,
                        "created_at": "2026-06-10T00:00:00Z",
                        "updated_at": "2026-06-20T00:00:00Z",
                        "pull_request": {"url": "..."},
                    },
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
                        {
                            "id": 500,
                            "run_attempt": 2,
                            "name": "ci",
                            "workflow_id": 9,
                            "event": "push",
                            "status": "completed",
                            "conclusion": "success",
                            "head_branch": "main",
                            "head_sha": "e" * 40,
                            "actor": {"login": "alice"},
                            "run_started_at": "2026-06-28T00:00:00Z",
                            "created_at": "2026-06-28T00:00:00Z",
                            "updated_at": "2026-06-28T01:00:00Z",
                        }
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


@pytest.mark.parametrize(
    "error",
    [
        pytest.param(
            {"type": "RATE_LIMIT", "code": "graphql_rate_limit", "message": "API quota exhausted."},
            id="typed_rate_limit",
        ),
        pytest.param(
            {"type": "SERVICE_UNAVAILABLE", "message": "API rate limit already exceeded."}, id="untyped_message"
        ),
    ],
)
@freezegun.freeze_time(_FROZEN)
def test_graphql_rate_limit_in_a_200_retries(http_mocker: HttpMocker, error: dict) -> None:
    """An exhausted GraphQL budget arrives as HTTP 200. GitHub types it
    RATE_LIMIT as well as RATE_LIMITED, so the throttle predicates match both
    spellings and fall back to the message — otherwise a throttle is mistaken
    for a query error and fails the stream."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_diff_stats_body()),
        [
            HttpResponse(body=json.dumps({"errors": [error]}), status_code=200, headers={"Retry-After": "0"}),
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
                                            "author": {
                                                "login": "alice",
                                                "databaseId": 4242,
                                                "email": "alice@example.com",
                                            },
                                            "mergedBy": {"login": "bob", "databaseId": 77},
                                            "reviewDecision": "APPROVED",
                                            "totalCommentsCount": 5,
                                            "isCrossRepository": False,
                                        }
                                    ],
                                }
                            }
                        }
                    }
                ),
                status_code=200,
            ),
        ],
    )

    output = read_stream(_CONNECTOR, "pull_request_diff_stats", config)

    assert not output.errors, "a rate-limit error must retry, not fail the stream"
    assert len(output.records) == 1


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
    node = {
        "id": "PVT_1",
        "number": 1,
        "title": "Roadmap",
        "shortDescription": None,
        "public": True,
        "closed": False,
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-06-01T00:00:00Z",
    }
    body = {
        "data": {
            "organization": {"projectsV2": {"pageInfo": {"hasNextPage": False, "endCursor": "c1"}, "nodes": [node]}}
        }
    }
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


def _authors_page(*rows: dict) -> HttpResponse:
    return HttpResponse(body=json.dumps({"items": list(rows), "next_page_token": None}), status_code=200)


def _author(email: str, sha: str, name: str = "Dev", committed: str = "2026-06-15T10:00:00+00:00") -> dict:
    return {
        "author_email": email,
        "author_name": name,
        "sample_sha": sha,
        "last_committed_date": committed,
        "commit_count": 3,
    }


def _commit_row(sha: str, committed: str) -> dict:
    return {
        "sha": sha,
        "message": "m",
        "committed_date": committed,
        "authored_date": committed,
        "author_name": "Dev",
        "author_email": "dev@example.com",
        "committer_name": "Dev",
        "committer_email": "dev@example.com",
        "parent_hashes": [],
        "is_merge": False,
        "is_in_default_branch": True,
        "patch_id": None,
    }


def _commits_page(*rows: dict) -> HttpResponse:
    return HttpResponse(body=json.dumps({"items": list(rows), "next_page_token": None}), status_code=200)


def _resolved_commit(sha: str, email: str, login: str, account_id: int) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "sha": sha,
                "node_id": "C_1",
                "commit": {"author": {"name": "Dev", "email": email}},
                "author": {"login": login, "id": account_id, "type": "User"},
                "committer": {"login": login, "id": account_id, "type": "User"},
                "parents": [],
            }
        ),
        status_code=200,
    )


@freezegun.freeze_time(_FROZEN)
def test_commit_authors_resolve_a_proxy_author_to_an_account(http_mocker: HttpMocker) -> None:
    """The proxy names the distinct authors; GitHub names the account behind
    each one. One vendor call per author, keyed on the git e-mail so the claim
    matches what the commit rows carry."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author("ada@example.com", "a" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/commits/{'a' * 40}", query_params=ANY_QUERY_PARAMS),
        _resolved_commit("a" * 40, "ada@example.com", "ada", 4242),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["author_email"] == "ada@example.com", "keyed on the git ident, not the profile"
    assert (rec["author_login"], rec["author_id"], rec["author_type"]) == ("ada", 4242, "User")
    assert rec["repo_full_name"] == "acme/app"
    assert rec["sample_sha"] == "a" * 40
    assert rec["unique_key"].endswith(":acme/app:author:ada@example.com")
    assert "commit" not in rec and "author" not in rec
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "commit_authors", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_commit_authors_drops_an_email_github_matches_to_nobody(http_mocker: HttpMocker) -> None:
    """GitHub answers `author: null` for an e-mail verified on no account — a
    CI or service identity. There is no account to claim the e-mail, so the
    row is dropped rather than stored as an unresolved one."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author("ci@build.local", "b" * 40, name="CI")),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/commits/{'b' * 40}", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                {
                    "sha": "b" * 40,
                    "commit": {"author": {"name": "CI", "email": "ci@build.local"}},
                    "author": None,
                    "committer": None,
                    "parents": [],
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    assert len(output.records) == 0, "an unmatched e-mail claims no account"


@freezegun.freeze_time(_FROZEN)
def test_commit_authors_state_carries_the_authors_since_date(http_mocker: HttpMocker) -> None:
    """The child carries a cursor so the author list's state persists. Without
    it every sync re-lists every author since the start date and re-resolves
    each one against GitHub; with it, the run emits the date the next run
    lists from."""
    config = GithubConfigBuilder().build()
    committed = "2026-06-15T10:00:00+00:00"
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author("ada@example.com", "a" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/commits/{'a' * 40}", query_params=ANY_QUERY_PARAMS),
        _resolved_commit("a" * 40, "ada@example.com", "ada", 4242),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    assert len(output.records) == 1
    # The record has no date of its own; it carries the author's last commit
    # date, which is what the cursor observes.
    assert output.records[0].record.data["last_committed_date"] == committed
    assert output.state_messages, "an incremental child must emit state"
    state = output.state_messages[-1].state.stream.stream_state.__dict__
    resumed = state["parent_state"]["repository_authors"]["state"]["last_committed_date"]
    assert _instant(resumed) == _instant(committed), f"parent state must carry the author's date: {state}"
    assert_records_conform(output.records, _CONNECTOR, "commit_authors", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_resumed_commit_authors_sync_lists_from_one_window_before_the_saved_date(http_mocker: HttpMocker) -> None:
    """The start date is a floor paid once. A run carrying state asks the proxy
    for the authors who committed since one lookback window before the saved
    date — a commit can be pushed days after it was made — so the GitHub
    lookups it spends follow recent activity rather than the whole history."""
    config = GithubConfigBuilder().build()
    # Far enough back that one window before the saved date is not clamped to it.
    config["github_start_date"] = "2026-01-01"
    one_window_before = "2026-05-15T10:00:00+00:00"
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author("ada@example.com", "a" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/commits/{'a' * 40}", query_params=ANY_QUERY_PARAMS),
        _resolved_commit("a" * 40, "ada@example.com", "ada", 4242),
    )
    first = read_stream(_CONNECTOR, "commit_authors", config)
    assert not first.errors
    state = [m.state for m in first.state_messages][-1:]

    resume_mocker = HttpMocker()
    with resume_mocker:
        resume_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
        # The proxy bound is inclusive, so the author on the boundary is listed again.
        resume_mocker.get(
            HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
            _authors_page(_author("ada@example.com", "a" * 40)),
        )
        resume_mocker.get(
            HttpRequest(f"{GH_URL}/repos/acme/app/commits/{'a' * 40}", query_params=ANY_QUERY_PARAMS),
            _resolved_commit("a" * 40, "ada@example.com", "ada", 4242),
        )

        second = read_stream(_CONNECTOR, "commit_authors", config, state=state)

        assert not second.errors
        since = [
            parse_qs(urlparse(r.url).query)["since"][0]
            for r in resume_mocker._mocker.request_history
            if r.url.startswith(f"{PROXY_URL}/v1/authors")
        ]
        assert since, "the resumed run must list authors"
        assert all(_instant(value) == _instant(one_window_before) for value in since), since
        lookups = [r.url for r in resume_mocker._mocker.request_history if "/commits/" in r.url]
        assert len(lookups) == 1, f"one author listed, one lookup: {lookups}"


@freezegun.freeze_time(_FROZEN)
def test_a_future_dated_commit_never_becomes_the_commits_cursor(http_mocker: HttpMocker) -> None:
    """A committer clock set ahead would otherwise become the saved cursor, and
    every later sync would ask for commits since a date that has not come. The
    row is dropped at the client and the cursor stays on the newest real date."""
    config = GithubConfigBuilder().build()
    sane, future = "2026-06-15T10:00:00+00:00", "2099-01-01T00:00:00+00:00"
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS),
        _commits_page(_commit_row("a" * 40, sane), _commit_row("b" * 40, future)),
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    assert [r.record.data["sha"] for r in output.records] == ["a" * 40]
    saved = json.dumps(output.state_messages[-1].state.stream.stream_state.__dict__)
    assert "2099" not in saved, f"the future date leaked into state: {saved}"
    assert "2026-06-15T10:00:00" in saved, f"the newest real date must be the cursor: {saved}"


@freezegun.freeze_time(_FROZEN)
def test_a_future_dated_author_never_becomes_the_authors_since(http_mocker: HttpMocker) -> None:
    """The author list is what a later sync bounds with `since`; an author whose
    last commit is dated ahead would push that bound past now and the list
    would come back empty forever. The author is dropped before the cursor
    sees them, and the others are still resolved."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(
            _author("ada@example.com", "a" * 40),
            _author("zed@example.com", "b" * 40, committed="2099-01-01T00:00:00+00:00"),
        ),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/commits/{'a' * 40}", query_params=ANY_QUERY_PARAMS),
        _resolved_commit("a" * 40, "ada@example.com", "ada", 4242),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    assert [r.record.data["author_email"] for r in output.records] == ["ada@example.com"]
    saved = json.dumps(output.state_messages[-1].state.stream.stream_state.__dict__)
    assert "2099" not in saved, f"the future date leaked into state: {saved}"


@freezegun.freeze_time(_FROZEN)
def test_roster_skips_a_repository_untouched_since_the_start_date(http_mocker: HttpMocker) -> None:
    """The proxy streams partition over this roster, so a repository whose last
    push predates the start date would be cloned for nothing: it has no commit,
    file change or author inside the window."""
    config = GithubConfigBuilder().build()
    stale = _repo() | {
        "id": 45,
        "full_name": "acme/dormant",
        "name": "dormant",
        "clone_url": "https://github.com/acme/dormant.git",
        "pushed_at": "2024-01-01T00:00:00Z",
    }
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_repo(), stale]), status_code=200),
    )
    # Every partition that IS walked yields a branch stamped with its clone URL.
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/branches", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                {
                    "items": [
                        {
                            "name": "main",
                            "head_sha": "f" * 40,
                            "head_committed_date": "2026-06-20T10:00:00Z",
                            "is_default": True,
                        }
                    ],
                    "next_page_token": None,
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "branches", config)

    assert not output.errors
    walked = {record.record.data["repository"] for record in output.records}
    assert walked == {"https://github.com/acme/app.git"}, (
        f"only repositories pushed since the start date may be walked: {walked}"
    )


def test_every_dated_stream_floors_on_the_start_date() -> None:
    """The start date is one bound in one direction: fetch everything since it,
    nothing before it. A stream that floors on anything else — a rolling window,
    an epoch — either discards data inside the window or fetches outside it, and
    both failures are silent."""
    manifest = load_manifest(_CONNECTOR)
    offenders: list[str] = []

    def walk(node, name="?"):
        if isinstance(node, dict):
            if node.get("type") == "DeclarativeStream":
                name = node.get("name", name)
                start = (node.get("incremental_sync") or {}).get("start_datetime") or {}
                declared = start.get("datetime") if isinstance(start, dict) else start
                if declared and "github_start_date" not in str(declared):
                    offenders.append(f"{name}: {declared}")
            for value in node.values():
                walk(value, name)
        elif isinstance(node, list):
            for item in node:
                walk(item, name)

    walk(manifest)
    assert not offenders, "streams flooring on something other than the start date: " + "; ".join(offenders)


@freezegun.freeze_time(_FROZEN)
def test_issue_timeline_multi_select_keeps_both_option_sets(http_mocker: HttpMocker) -> None:
    """A multi-select change states its whole option set on both sides and
    leaves previousValue/newValue null. Reading only the scalars would record
    the event with an empty before and after — a change that says nothing."""
    config = GithubConfigBuilder().build()
    _mock_no_boards(http_mocker)
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([{"number": 7, "updated_at": "2026-06-20T00:00:00Z"}]), status_code=200),
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
                                            "__typename": "IssueFieldChangedEvent",
                                            "id": "IF_MS",
                                            "createdAt": "2026-06-12T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "issueField": {"id": "IFMS_1", "name": "Platforms"},
                                            "previousValue": None,
                                            "newValue": None,
                                            "previousOptions": [{"name": "iOS"}],
                                            "newOptions": [{"name": "iOS"}, {"name": "Android"}],
                                        }
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
    rec = output.records[0].record.data
    assert rec["field_id"] == "IFMS_1"
    assert json.loads(rec["prev_options_json"]) == ["iOS"]
    assert json.loads(rec["new_options_json"]) == ["iOS", "Android"]
    assert (rec["prev_value"], rec["new_value"]) == ("", ""), "a multi-select states nothing in the scalars"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "issue_timeline_events", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_issues_hoist_the_native_field_values(http_mocker: HttpMocker) -> None:
    """A native issue field set at creation and never changed produces no
    timeline event, so this snapshot is the only statement of its value that
    ever reaches bronze. The nested original stays out."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    values = [
        {"issue_field": {"id": "IFN_1", "name": "Estimated Efforts m*d"}, "value": 5},
        {"issue_field": {"id": "IFD_1", "name": "Target date"}, "value": "2026-07-01"},
    ]
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {
                        "id": 900,
                        "number": 7,
                        "state": "open",
                        "title": "Ship it",
                        "created_at": "2026-06-01T00:00:00Z",
                        "updated_at": "2026-06-20T00:00:00Z",
                        "type": {"node_id": "IT_bug", "name": "Bug"},
                        "issue_field_values": values,
                    }
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "issues", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert json.loads(rec["issue_field_values_json"]) == values
    assert (rec["issue_type"], rec["issue_type_id"]) == ("Bug", "IT_bug")
    assert "issue_field_values" not in rec, "the nested original is hoisted, not duplicated"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "issues", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_issues_without_native_fields_emit_an_empty_list(http_mocker: HttpMocker) -> None:
    """An organization that defines no issue fields answers without the key at
    all. That must read as "no values", not as a literal None."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([{"id": 901, "number": 8, "updated_at": "2026-06-20T00:00:00Z"}]), status_code=200
        ),
    )

    output = read_stream(_CONNECTOR, "issues", config)

    assert not output.errors
    assert json.loads(output.records[0].record.data["issue_field_values_json"]) == []
    _no_literal_none(output.records)


def _issue_fields_body(cursor: str | None = None) -> dict:
    return _graphql_body("issue_fields", {"org": "acme"}, cursor)


def _issue_types_body(cursor: str | None = None) -> dict:
    return _graphql_body("issue_types", {"org": "acme"}, cursor)


@freezegun.freeze_time(_FROZEN)
def test_issue_fields_catalogue_carries_identity_and_options(http_mocker: HttpMocker) -> None:
    """History names a field by node id; without this catalogue that id
    resolves to nothing and an operator binding a field to a metric role has
    no list of real identifiers to bind against."""
    config = GithubConfigBuilder().build()
    nodes = [
        {
            "__typename": "IssueFieldSingleSelect",
            "id": "IFSS_1",
            "fullDatabaseId": 111,
            "name": "Priority",
            "dataType": "SINGLE_SELECT",
            "options": [{"id": "o1", "name": "High"}, {"id": "o2", "name": "Low"}],
        },
        {
            "__typename": "IssueFieldNumber",
            "id": "IFN_1",
            "fullDatabaseId": 222,
            "name": "Estimated Efforts m*d",
            "dataType": "NUMBER",
        },
        {
            "__typename": "IssueFieldMultiSelect",
            "id": "IFMS_1",
            "name": "Platforms",
            "dataType": "MULTI_SELECT",
            "options": [{"id": "o3", "name": "iOS"}],
        },
    ]
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_fields_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "organization": {
                            "issueFields": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": nodes}
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "issue_fields", config)

    assert not output.errors
    by_id = {r.record.data["field_id"]: r.record.data for r in output.records}
    assert set(by_id) == {"IFSS_1", "IFN_1", "IFMS_1"}
    assert by_id["IFSS_1"]["data_type"] == "SINGLE_SELECT"
    assert [o["name"] for o in json.loads(by_id["IFSS_1"]["options_json"])] == ["High", "Low"]
    assert by_id["IFMS_1"]["is_multi"] is True
    assert by_id["IFN_1"]["is_multi"] is False
    assert json.loads(by_id["IFN_1"]["options_json"]) == [], "a number field admits no options"
    # The REST issue payload names a field by its numeric id and the timeline
    # by node id; both must resolve to this one catalogue row.
    assert by_id["IFN_1"]["field_database_id"] == "222"
    assert by_id["IFSS_1"]["field_database_id"] == "111"
    assert by_id["IFN_1"]["unique_key"].endswith(":acme:issue_field:IFN_1")
    assert by_id["IFN_1"]["tenant_id"] == config["insight_tenant_id"]
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "issue_fields", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_issue_fields_paginate_with_the_cursor_in_the_body(http_mocker: HttpMocker) -> None:
    """A catalogue larger than one page must not truncate silently — the whole
    point of the stream is that every identifier history can name is present."""
    config = GithubConfigBuilder().build()
    page1 = {
        "data": {
            "organization": {
                "issueFields": {
                    "pageInfo": {"hasNextPage": True, "endCursor": "c1"},
                    "nodes": [{"__typename": "IssueFieldText", "id": "IFT_1", "name": "Notes", "dataType": "TEXT"}],
                }
            }
        }
    }
    page2 = {
        "data": {
            "organization": {
                "issueFields": {
                    "pageInfo": {"hasNextPage": False, "endCursor": "c2"},
                    "nodes": [
                        {"__typename": "IssueFieldDate", "id": "IFD_1", "name": "Target date", "dataType": "DATE"}
                    ],
                }
            }
        }
    }
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_fields_body()),
        HttpResponse(body=json.dumps(page1), status_code=200),
    )
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_fields_body(cursor="c1")),
        HttpResponse(body=json.dumps(page2), status_code=200),
    )

    output = read_stream(_CONNECTOR, "issue_fields", config)

    assert not output.errors
    assert {r.record.data["field_id"] for r in output.records} == {"IFT_1", "IFD_1"}


@freezegun.freeze_time(_FROZEN)
def test_issue_types_catalogue_gives_the_type_a_stable_key(http_mocker: HttpMocker) -> None:
    """The issue payload states its type by display name only, so a rename
    orphans every mapping built on the name alone."""
    config = GithubConfigBuilder().build()
    nodes = [
        {"id": "IT_task", "name": "Task", "description": "A specific piece of work", "isEnabled": True},
        {"id": "IT_bug", "name": "Bug", "description": None, "isEnabled": False},
    ]
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_types_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "organization": {
                            "issueTypes": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": nodes}
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "issue_types", config)

    assert not output.errors
    by_id = {r.record.data["issue_type_id"]: r.record.data for r in output.records}
    assert by_id["IT_task"]["issue_type_name"] == "Task"
    assert by_id["IT_task"]["is_enabled"] is True
    assert by_id["IT_bug"]["is_enabled"] is False
    assert by_id["IT_bug"]["description"] == "", "a null description is empty, never a literal None"
    assert by_id["IT_bug"]["unique_key"].endswith(":acme:issue_type:IT_bug")
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "issue_types", strict=True)


def _projects_parent_body(cursor: str | None = None) -> dict:
    """The board enumerator lives in `definitions`, not `streams` — the two
    board substreams share it, so its body is built from there."""
    manifest = load_manifest(_CONNECTOR)
    parent = manifest["definitions"]["projects_all_parent"]
    body = dict(parent["retriever"]["requester"]["request_body_json"])
    body["query"] = body["query"].rstrip("\n")
    body["variables"] = {"org": "acme"}
    if cursor is not None:
        body["variables"]["cursor"] = cursor
    return body


def _mock_projects_parent(http_mocker: HttpMocker, nodes: list[dict]) -> None:
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_projects_parent_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "organization": {
                            "projectsV2": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": nodes}
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )


def _project_card_issues_body(project_id: str, cursor: str | None = None) -> dict:
    """The board-card parent of the issue timeline also lives in `definitions`."""
    manifest = load_manifest(_CONNECTOR)
    parent = manifest["definitions"]["project_card_issues_parent"]
    body = dict(parent["retriever"]["requester"]["request_body_json"])
    body["query"] = body["query"].rstrip("\n")
    body["variables"] = {"project": project_id}
    if cursor is not None:
        body["variables"]["cursor"] = cursor
    return body


def _mock_no_boards(http_mocker: HttpMocker) -> None:
    """The issue timeline has a board-side parent as well as the issue window.
    A test that is not about boards declares the org has none, so only the
    window produces partitions."""
    _mock_projects_parent(http_mocker, [])


def _project_fields_body(project_id: str, cursor: str | None = None) -> dict:
    return _graphql_body("project_fields", {"project": project_id}, cursor)


def _project_items_body(project_id: str, cursor: str | None = None) -> dict:
    return _graphql_body("project_items", {"project": project_id}, cursor)


@freezegun.freeze_time(_FROZEN)
def test_project_fields_are_scoped_to_their_own_board(http_mocker: HttpMocker) -> None:
    """A same-named Status in two boards is two different fields. Collecting
    the catalogue per board is what keeps a status resolvable at all — and what
    stops one board's option name being read as another's."""
    config = GithubConfigBuilder().build()
    _mock_projects_parent(http_mocker, [{"id": "PVT_1", "number": 40}, {"id": "PVT_2", "number": 41}])
    for project_id, option_name in (("PVT_1", "In progress"), ("PVT_2", "10%")):
        http_mocker.post(
            HttpRequest(f"{GH_URL}/graphql", body=_project_fields_body(project_id)),
            HttpResponse(
                body=json.dumps(
                    {
                        "data": {
                            "node": {
                                "fields": {
                                    "pageInfo": {"hasNextPage": False, "endCursor": None},
                                    "nodes": [
                                        {
                                            "__typename": "ProjectV2SingleSelectField",
                                            "id": f"PVTSSF_{project_id}",
                                            "databaseId": 1,
                                            "name": "Status",
                                            "dataType": "SINGLE_SELECT",
                                            "updatedAt": "2026-06-02T00:00:00Z",
                                            "isIssueField": False,
                                            # Boards created from one template
                                            # inherit its option ids, then
                                            # rename the options independently.
                                            "options": [{"id": "shared-option", "name": option_name}],
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

    output = read_stream(_CONNECTOR, "project_fields", config)

    assert not output.errors
    by_field = {r.record.data["field_id"]: r.record.data for r in output.records}
    assert set(by_field) == {"PVTSSF_PVT_1", "PVTSSF_PVT_2"}
    assert by_field["PVTSSF_PVT_1"]["project_number"] == 40
    assert by_field["PVTSSF_PVT_2"]["project_number"] == 41
    one = json.loads(by_field["PVTSSF_PVT_1"]["options_json"])
    two = json.loads(by_field["PVTSSF_PVT_2"]["options_json"])
    assert one[0]["id"] == two[0]["id"], "the fixture shares an option id across boards on purpose"
    assert one[0]["name"] != two[0]["name"], "and the names diverge, so the id alone identifies nothing"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "project_fields", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_project_fields_mark_the_issue_mirrors(http_mocker: HttpMocker) -> None:
    """A built-in board field is a projection of the issue. Read as a value it
    would compete with the issue's own field for the same fact, so the
    catalogue has to say which is which."""
    config = GithubConfigBuilder().build()
    _mock_projects_parent(http_mocker, [{"id": "PVT_1", "number": 40}])
    nodes = [
        {"__typename": "ProjectV2Field", "id": "F_title", "name": "Title", "dataType": "TITLE"},
        {"__typename": "ProjectV2Field", "id": "F_assignees", "name": "Assignees", "dataType": "ASSIGNEES"},
        {"__typename": "ProjectV2Field", "id": "F_est", "name": "Estimate", "dataType": "NUMBER"},
        {
            "__typename": "ProjectV2IterationField",
            "id": "F_sprint",
            "name": "Sprint",
            "dataType": "ITERATION",
            "configuration": {
                "duration": 14,
                "startDay": 1,
                "iterations": [{"id": "it1", "title": "S1"}],
                "completedIterations": [],
            },
        },
    ]
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_project_fields_body("PVT_1")),
        HttpResponse(
            body=json.dumps(
                {"data": {"node": {"fields": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": nodes}}}}
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "project_fields", config)

    assert not output.errors
    mirror = {r.record.data["field_id"]: r.record.data["is_mirror"] for r in output.records}
    assert mirror == {"F_title": True, "F_assignees": True, "F_est": False, "F_sprint": False}
    sprint = next(r.record.data for r in output.records if r.record.data["field_id"] == "F_sprint")
    assert json.loads(sprint["configuration_json"])["duration"] == 14
    _no_literal_none(output.records)


@freezegun.freeze_time(_FROZEN)
def test_project_fields_key_is_the_field_so_bronze_holds_the_present(http_mocker: HttpMocker) -> None:
    """Bronze states what is true now and the ReplacingMergeTree collapses each
    re-collection onto it. History belongs to the SCD2 snapshot downstream, not
    to the row key: keying by day would keep every version permanently current
    (ADR-0001, ADR-0004)."""
    config = GithubConfigBuilder().build()
    _mock_projects_parent(http_mocker, [{"id": "PVT_1", "number": 40}])
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_project_fields_body("PVT_1")),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "node": {
                            "fields": {
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": [
                                    {
                                        "__typename": "ProjectV2Field",
                                        "id": "F_est",
                                        "name": "Estimate",
                                        "dataType": "NUMBER",
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

    output = read_stream(_CONNECTOR, "project_fields", config)

    assert not output.errors
    key = output.records[0].record.data["unique_key"]
    assert key.endswith(":project:PVT_1:field:F_est"), key
    assert "2026-07-01" not in key, "a collection day in the key would defeat the RMT collapse"
    assert "snapshot_date" not in output.records[0].record.data


def _card(item_id: str, updated_at: str, content: dict | None = None) -> dict:
    return {
        "id": item_id,
        "fullDatabaseId": 5551,
        "type": "ISSUE",
        "createdAt": "2026-06-02T00:00:00Z",
        "updatedAt": updated_at,
        "isArchived": False,
        "creator": {"login": "alice", "databaseId": 1001},
        "content": content
        if content is not None
        else {"__typename": "Issue", "number": 7, "repository": {"nameWithOwner": "acme/app"}},
        "fieldValues": {
            "nodes": [
                {
                    "__typename": "ProjectV2ItemFieldSingleSelectValue",
                    "optionId": "shared-option",
                    "name": "In progress",
                    "updatedAt": updated_at,
                    "field": {"id": "PVTSSF_PVT_1", "name": "Status", "dataType": "SINGLE_SELECT"},
                }
            ]
        },
    }


@freezegun.freeze_time(_FROZEN)
def test_project_items_drop_a_draft_card_but_keep_an_unreadable_one(http_mocker: HttpMocker) -> None:
    """A draft card has no issue behind it and no identity outside the board, so
    it can join nothing downstream. A card with null content is a different
    thing entirely — its issue lives where the token cannot look — and dropping
    both together would hide a coverage gap behind a board note."""
    config = GithubConfigBuilder().build()
    _mock_projects_parent(http_mocker, [{"id": "PVT_1", "number": 40}])
    nodes = [
        _card("PVTI_issue", "2026-06-20T10:00:00Z"),
        _card("PVTI_draft", "2026-06-20T10:00:00Z", content={"__typename": "DraftIssue"}),
        _card("PVTI_none", "2026-06-20T10:00:00Z", content=None),
    ]
    # `None` content is a card whose issue the token cannot see, not a draft.
    nodes[2]["content"] = None
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_project_items_body("PVT_1")),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "node": {
                            "items": {
                                "totalCount": 3,
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": nodes,
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "project_items", config)

    assert not output.errors
    kept = {r.record.data["item_id"]: r.record.data for r in output.records}
    assert set(kept) == {"PVTI_issue", "PVTI_none"}, "the draft goes, the unreadable card stays"
    assert kept["PVTI_issue"]["content_type"] == "Issue"
    assert kept["PVTI_none"]["content_type"] == "", "unknown content is empty, never a literal None"
    assert kept["PVTI_none"]["content_number"] == 0
    assert kept["PVTI_none"]["content_repo_full_name"] == ""


@freezegun.freeze_time(_FROZEN)
def test_project_item_key_is_the_card_so_a_re_read_collapses(http_mocker: HttpMocker) -> None:
    """The cursor re-reads a day on every sync by design. That is free only
    while the key is the card itself: the re-read collapses onto the row
    already there instead of appending a version."""
    config = GithubConfigBuilder().build()
    _mock_projects_parent(http_mocker, [{"id": "PVT_1", "number": 40}])
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_project_items_body("PVT_1")),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "node": {
                            "items": {
                                "totalCount": 2,
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": [
                                    _card("PVTI_1", "2026-06-20T10:00:00Z"),
                                    _card("PVTI_1", "2026-06-21T09:00:00Z"),
                                ],
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "project_items", config)

    assert not output.errors
    keys = [r.record.data["unique_key"] for r in output.records]
    assert keys[0].endswith(":project:PVT_1:item:PVTI_1")
    assert keys[0] == keys[1], "the same card twice is the same row, whatever day it moved"
    first = output.records[0].record.data
    assert first["content_number"] == 7
    assert first["content_repo_full_name"] == "acme/app"
    assert json.loads(first["field_values_json"])[0]["optionId"] == "shared-option"


@freezegun.freeze_time(_FROZEN)
def test_project_items_sweep_the_whole_board_with_no_filter(http_mocker: HttpMocker) -> None:
    """Board membership is a snapshot, so the cards are swept rather than
    windowed: a card untouched since the last sync is still collected, and the
    request carries no `updated:>=` qualifier that could leave it behind."""
    config = GithubConfigBuilder().build()
    _mock_projects_parent(http_mocker, [{"id": "PVT_1", "number": 40}])

    body = _project_items_body("PVT_1")
    assert "query" not in body["variables"], "the sweep must not send a filter variable"
    assert "$q" not in body["query"], "the sweep must not declare a filter argument"

    # A card long untouched shares the page with a fresh one, and the page after
    # it is reached only by following the cursor to exhaustion.
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=body),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "node": {
                            "items": {
                                "totalCount": 3,
                                "pageInfo": {"hasNextPage": True, "endCursor": "CUR1"},
                                "nodes": [
                                    _card("PVTI_old", "2019-01-01T00:00:00Z"),
                                    _card("PVTI_new", "2026-06-30T00:00:00Z"),
                                ],
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_project_items_body("PVT_1", "CUR1")),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "node": {
                            "items": {
                                "totalCount": 3,
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": [_card("PVTI_last", "2018-05-05T00:00:00Z")],
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "project_items", config)

    assert not output.errors
    assert [r.record.data["item_id"] for r in output.records] == ["PVTI_old", "PVTI_new", "PVTI_last"]


@freezegun.freeze_time(_FROZEN)
def test_a_board_reports_the_card_count_a_sweep_is_checked_against(http_mocker: HttpMocker) -> None:
    """The cards a sweep keeps are every card except the drafts it drops, so
    the board has to state both numbers for the sweep to be checkable at all."""
    config = GithubConfigBuilder().build()
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_graphql_body("projects_v2", {"org": "acme"})),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "organization": {
                            "projectsV2": {
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": [
                                    {
                                        "id": "PVT_1",
                                        "number": 40,
                                        "title": "Board",
                                        "shortDescription": None,
                                        "public": True,
                                        "closed": False,
                                        "createdAt": "2020-01-01T00:00:00Z",
                                        "updatedAt": "2026-06-30T00:00:00Z",
                                        "all_items": {"totalCount": 730},
                                        "draft_items": {"totalCount": 6},
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

    output = read_stream(_CONNECTOR, "projects_v2", config)

    assert not output.errors
    row = output.records[0].record.data
    assert row["total_items"] == 730
    assert row["draft_items"] == 6
    assert "all_items" not in row, "the nested count shape must not reach bronze"


@freezegun.freeze_time(_FROZEN)
def test_status_change_names_the_board_it_happened_on(http_mocker: HttpMocker) -> None:
    """Every board defines its own Status field, so a status event without its
    board cannot be resolved. An issue on several boards interleaves all their
    events in this one timeline."""
    config = GithubConfigBuilder().build()
    _mock_no_boards(http_mocker)
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_repo()]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([{"id": 901, "number": 7, "updated_at": "2026-06-20T00:00:00Z"}]), status_code=200
        ),
    )
    nodes = [
        {
            "__typename": "ProjectV2ItemStatusChangedEvent",
            "id": "E_status_a",
            "createdAt": "2026-06-10T00:00:00Z",
            "actor": {"login": "alice", "databaseId": 1001},
            "previousStatus": "Todo",
            "status": "In progress",
            "project": {"id": "PVT_1", "number": 40},
            "wasAutomated": False,
        },
        {
            "__typename": "ProjectV2ItemStatusChangedEvent",
            "id": "E_status_b",
            "createdAt": "2026-06-11T00:00:00Z",
            "actor": {"login": "bot", "databaseId": None},
            "previousStatus": "Todo",
            "status": "In progress",
            "project": {"id": "PVT_2", "number": 41},
            "wasAutomated": True,
        },
        {
            "__typename": "AddedToProjectV2Event",
            "id": "E_added",
            "createdAt": "2026-06-09T00:00:00Z",
            "actor": {"login": "alice", "databaseId": 1001},
            "project": {"id": "PVT_1", "number": 40},
        },
        {
            "__typename": "RemovedFromProjectV2Event",
            "id": "E_removed",
            "createdAt": "2026-06-12T00:00:00Z",
            "actor": {"login": "alice", "databaseId": 1001},
            "project": {"id": "PVT_2", "number": 41},
        },
    ]
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_timeline_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "repository": {
                            "issue": {
                                "timelineItems": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": nodes}
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
    by_event = {r.record.data["event_id"]: r.record.data for r in output.records}
    assert set(by_event) == {"E_status_a", "E_status_b", "E_added", "E_removed"}
    # Two boards, one issue: same status name, different boards.
    assert by_event["E_status_a"]["new_value"] == by_event["E_status_b"]["new_value"] == "In progress"
    assert by_event["E_status_a"]["project_id"] == "PVT_1"
    assert by_event["E_status_b"]["project_id"] == "PVT_2"
    assert by_event["E_status_a"]["project_number"] == 40
    assert by_event["E_status_a"]["was_automated"] is False
    assert by_event["E_status_b"]["was_automated"] is True, "a workflow's move is not a person's"
    # Membership states the board and nothing else — the event type is the fact.
    assert by_event["E_added"]["project_id"] == "PVT_1"
    assert by_event["E_removed"]["project_id"] == "PVT_2"
    assert by_event["E_added"]["new_value"] == ""
    _no_literal_none(output.records)


def _issue_links_body(cursor: str | None = None) -> dict:
    return _graphql_body("issue_links", {"owner": "acme", "name": "app"}, cursor)


@freezegun.freeze_time(_FROZEN)
def test_a_moved_card_brings_its_issue_back_into_the_timeline_walk(http_mocker: HttpMocker) -> None:
    """Moving a card leaves the issue's own `updated_at` alone, so the issue
    window never yields it again. The board-card parent is the second way in,
    and without it the status change behind the move is unreachable for good."""
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    # The issue itself was not touched, so its own window answers with nothing.
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([]), status_code=200),
    )
    _mock_projects_parent(http_mocker, [{"id": "PVT_1", "number": 5}])
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_project_card_issues_body("PVT_1")),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "node": {
                            "items": {
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": [
                                    {
                                        "updated_at": "2026-06-25T00:00:00Z",
                                        "content": {
                                            "__typename": "Issue",
                                            "number": 7,
                                            "repository": {"nameWithOwner": "acme/app"},
                                        },
                                    },
                                    # A draft card has no issue to walk.
                                    {"updated_at": "2026-06-25T00:00:00Z", "content": {"__typename": "DraftIssue"}},
                                ],
                            }
                        }
                    }
                }
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
                                            "id": "PS_9",
                                            "createdAt": "2026-06-25T00:00:00Z",
                                            "actor": {"login": "alice"},
                                            "previousStatus": "In Progress",
                                            "status": "Done",
                                            "projectV2": {"id": "PVT_1", "number": 5},
                                        }
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
    assert [r.record.data["event_id"] for r in output.records] == ["PS_9"]
    assert output.records[0].record.data["item_number"] == 7
    assert output.records[0].record.data["repo_full_name"] == "acme/app"


@freezegun.freeze_time(_FROZEN)
def test_link_events_collapse_six_payload_shapes_into_one_target(http_mocker: HttpMocker) -> None:
    """Each link event names the other end under its own key — subIssue,
    parent, blockingIssue, blockedIssue, subject, canonical. Downstream has to
    fold adds against removes, which it cannot do while the target's location
    depends on which of the twelve types carried it."""
    config = GithubConfigBuilder().build()
    _mock_no_boards(http_mocker)
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_repo()]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{GH_URL}/repos/acme/app/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([{"id": 901, "number": 7, "updated_at": "2026-06-20T00:00:00Z"}]), status_code=200
        ),
    )
    actor = {"login": "alice", "databaseId": 1001}
    other = {"number": 11, "repository": {"nameWithOwner": "acme/app"}}
    cross = {"number": 3, "repository": {"nameWithOwner": "acme/other"}}
    nodes = [
        {
            "__typename": "SubIssueAddedEvent",
            "id": "L1",
            "createdAt": "2026-06-01T00:00:00Z",
            "actor": actor,
            "subIssue": other,
        },
        {
            "__typename": "SubIssueRemovedEvent",
            "id": "L2",
            "createdAt": "2026-06-02T00:00:00Z",
            "actor": actor,
            "subIssue": other,
        },
        {
            "__typename": "ParentIssueAddedEvent",
            "id": "L3",
            "createdAt": "2026-06-03T00:00:00Z",
            "actor": actor,
            "parent": cross,
        },
        {
            "__typename": "BlockedByAddedEvent",
            "id": "L4",
            "createdAt": "2026-06-04T00:00:00Z",
            "actor": actor,
            "blockingIssue": other,
        },
        {
            "__typename": "BlockingAddedEvent",
            "id": "L5",
            "createdAt": "2026-06-05T00:00:00Z",
            "actor": actor,
            "blockedIssue": other,
        },
        {
            "__typename": "ConnectedEvent",
            "id": "L6",
            "createdAt": "2026-06-06T00:00:00Z",
            "actor": actor,
            "isCrossRepository": False,
            "subject": {"__typename": "PullRequest", "number": 42, "repository": {"nameWithOwner": "acme/app"}},
        },
        {
            "__typename": "MarkedAsDuplicateEvent",
            "id": "L7",
            "createdAt": "2026-06-07T00:00:00Z",
            "actor": actor,
            "isCrossRepository": True,
            "canonical": {"__typename": "Issue", "number": 9, "repository": {"nameWithOwner": "acme/other"}},
        },
    ]
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_timeline_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "repository": {
                            "issue": {
                                "timelineItems": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": nodes}
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
    by_event = {r.record.data["event_id"]: r.record.data for r in output.records}
    assert set(by_event) == {"L1", "L2", "L3", "L4", "L5", "L6", "L7"}
    for event_id in ("L1", "L2", "L4", "L5"):
        assert by_event[event_id]["link_target_number"] == 11, event_id
        assert by_event[event_id]["link_target_repo_full_name"] == "acme/app", event_id
        assert by_event[event_id]["link_target_type"] == "Issue", event_id
    # A hierarchy link may cross repositories, so the target's repository is
    # part of its identity and not decoration.
    assert by_event["L3"]["link_target_repo_full_name"] == "acme/other"
    # Only the connect and duplicate pairs state a __typename of their own.
    assert by_event["L6"]["link_target_type"] == "PullRequest"
    assert by_event["L6"]["link_target_number"] == 42
    assert by_event["L7"]["link_target_type"] == "Issue"
    assert by_event["L7"]["is_cross_repository"] is True
    # An event that is not a link leaves the columns empty rather than guessing.
    _no_literal_none(output.records)


@freezegun.freeze_time(_FROZEN)
def test_issue_links_snapshot_carries_every_link_set(http_mocker: HttpMocker) -> None:
    """A pull request that closes an issue has no reliable event — the timeline
    may call it a connection, a cross-reference that claims it will NOT close
    the issue, or say nothing. Observing the connection is the only way to know
    it, so the snapshot must carry it alongside the sets the timeline does
    cover."""
    config = GithubConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_repo()]), status_code=200),
    )
    node = {
        "number": 7,
        "updatedAt": "2026-06-20T10:00:00Z",
        "parent": {"number": 1, "repository": {"nameWithOwner": "acme/app"}},
        "subIssues": {
            "totalCount": 2,
            "nodes": [
                {"number": 8, "repository": {"nameWithOwner": "acme/app"}},
                {"number": 9, "repository": {"nameWithOwner": "acme/other"}},
            ],
        },
        "blockedBy": {"totalCount": 1, "nodes": [{"number": 5, "repository": {"nameWithOwner": "acme/app"}}]},
        "blocking": {"totalCount": 0, "nodes": []},
        "closedByPullRequestsReferences": {
            "totalCount": 1,
            "nodes": [{"number": 42, "repository": {"nameWithOwner": "acme/app"}}],
        },
    }
    http_mocker.post(
        HttpRequest(f"{GH_URL}/graphql", body=_issue_links_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "repository": {
                            "issues": {"pageInfo": {"hasNextPage": False, "endCursor": None}, "nodes": [node]}
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "issue_links", config)

    assert not output.errors
    row = output.records[0].record.data
    assert row["item_number"] == 7
    assert row["repo_full_name"] == "acme/app"
    assert json.loads(row["parent_json"])["number"] == 1
    assert [n["number"] for n in json.loads(row["sub_issues_json"])] == [8, 9]
    assert [n["number"] for n in json.loads(row["blocked_by_json"])] == [5]
    assert json.loads(row["blocking_json"]) == [], "an empty set is empty, never absent"
    assert [n["number"] for n in json.loads(row["closed_by_pull_requests_json"])] == [42]
    # The vendor's own counts travel with the sets: a nested connection cannot
    # be paginated here, so the count is the only way a truncated set is
    # visible at all.
    assert row["sub_issues_total"] == 2
    assert row["blocking_total"] == 0
    assert row["closed_by_pull_requests_total"] == 1
    assert row["unique_key"].endswith(":acme/app:issue_links:7")
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "issue_links", strict=True)


_PROXY_RESET_ACTIONS = {
    "/v1/commits": "SPLIT_USING_CURSOR",
    "/v1/file-changes": "SPLIT_USING_CURSOR",
    "/v1/branches": "RESET",
    "/v1/authors": "RESET",
}


@freezegun.freeze_time(_FROZEN)
def test_a_proxy_401_is_the_proxy_token_and_fails_as_a_config_error(http_mocker: HttpMocker) -> None:
    config = GithubConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/branches", query_params=ANY_QUERY_PARAMS), HttpResponse(body="", status_code=401)
    )

    output = read_stream(_CONNECTOR, "branches", config, expecting_exception=True)

    assert output.errors
    assert output.errors[-1].trace.error.failure_type == FailureType.config_error


def test_every_proxy_request_carries_the_repository_size_hint() -> None:
    """The proxy reserves cache headroom from the hint instead of its per-repository
    cap; a proxy requester without it, or a proxy parent that does not pass the size
    along, silently falls back to the cap."""
    retrievers = _proxy_retrievers(load_manifest(_CONNECTOR)["streams"])
    assert retrievers
    for retriever in retrievers:
        hint = (retriever["requester"].get("request_headers") or {}).get("X-Repo-Size-Hint", "")
        assert "extra_fields.get('size')" in hint, retriever["requester"]["path"]
        parents = retriever["partition_router"]["parent_stream_configs"]
        for parent in parents:
            if parent.get("partition_field") == "repo_clone_url":
                assert ["size"] in (parent.get("extra_fields") or []), retriever["requester"]["path"]


def _proxy_retrievers(node, out=None):
    """Every SimpleRetriever whose requester targets the git proxy."""
    if out is None:
        out = []
    if isinstance(node, dict):
        requester = node.get("requester", {})
        if node.get("type") == "SimpleRetriever" and "git_proxy_url" in str(requester.get("url_base", "")):
            out.append(node)
        for value in node.values():
            _proxy_retrievers(value, out)
    elif isinstance(node, list):
        for item in node:
            _proxy_retrievers(item, out)
    return out


def test_a_superseded_proxy_snapshot_restarts_the_walk_instead_of_failing_it() -> None:
    """A 409 means the page token points into a snapshot the proxy no longer
    holds. Failing the partition freezes its cursor until the next run; a
    pagination reset restarts the walk, and a walk the proxy orders by the
    cursor restarts from the last value already seen. Commits and file
    changes come out ordered by committed_date; branches and authors carry
    no such order, so their restart is from the first page."""
    manifest = load_manifest(_CONNECTOR)
    retrievers = _proxy_retrievers(manifest["streams"])
    assert {r["requester"]["path"] for r in retrievers} == set(_PROXY_RESET_ACTIONS)
    for retriever in retrievers:
        path = retriever["requester"]["path"]
        filters = retriever["requester"]["error_handler"]["response_filters"]
        on_409 = [f["action"] for f in filters if 409 in f.get("http_codes", [])]
        assert on_409 == ["RESET_PAGINATION"], f"{path}: a 409 must reset pagination, got {on_409}"
        reset = retriever.get("pagination_reset")
        assert reset == {"type": "PaginationReset", "action": _PROXY_RESET_ACTIONS[path]}, f"{path}: {reset}"
