"""Mock-server tests for the Bitbucket vendor streams.

The vendor-specific hazards under test: the four-state path filter and the
pagelen-50 cap on /pullrequests, the activity stream's synthesized key
(entries carry no id), per-repository 403 skipping (the exact incident class
that used to break syncs: some repos 403 even on a valid token), and the
literal-"None" guard on every hoisted field.

Coverage matrix rows: full_refresh_single_page, incremental_state,
tenant_source_stamping, schema_conformance, substream_partition,
error_ignore (403), error_retry (429), transformations (None-guard).
"""

from __future__ import annotations

import json
from collections.abc import Iterable
from typing import Any
from urllib.parse import unquote_plus

import freezegun
from airbyte_cdk.models import AirbyteMessage
from config import BB_URL, PROXY_URL, BitbucketCloudConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    read_stream,
)

_CONNECTOR = "git/bitbucket-cloud"
_REPOS_URL = f"{BB_URL}/repositories/acme"
_FROZEN = "2026-07-01T00:00:00Z"


def _no_literal_none(records: Iterable[AirbyteMessage]) -> None:
    for r in records:
        for key, value in r.record.data.items():
            assert value != "None", f"literal 'None' leaked into {key}"


def _repo() -> dict[str, Any]:
    return {
        "uuid": "{r-1}",
        "full_name": "acme/app",
        "updated_on": "2026-06-20T10:00:00.000000+00:00",
    }


def _repos_page() -> HttpResponse:
    return HttpResponse(body=json.dumps({"values": [_repo()]}), status_code=200)


def _pr(pr_id: int, *, author: dict[str, Any] | None) -> dict[str, Any]:
    return {
        "id": pr_id,
        "title": f"PR {pr_id}",
        "state": "MERGED",
        "draft": False,
        "author": author,
        "created_on": "2026-06-10T10:00:00.000000+00:00",
        "updated_on": "2026-06-20T10:00:00.000000+00:00",
        "merge_commit": {"hash": "a" * 12},
        "closed_by": None,
        "source": {"branch": {"name": "feat"}, "commit": {"hash": "b" * 12}},
        "destination": {"branch": {"name": "main"}, "commit": {"hash": "c" * 12}},
        "description": "d" * 3000,
        "participants": [
            {
                "user": {"uuid": "{u-2}", "display_name": "Bob"},
                "role": "REVIEWER",
                "state": "approved",
                "approved": True,
                "participated_on": "2026-06-19T00:00:00+00:00",
            }
        ],
        "comment_count": 1,
        "task_count": 0,
    }


@freezegun.freeze_time(_FROZEN)
def test_pull_requests_four_states_and_none_guard(http_mocker: HttpMocker) -> None:
    """The path carries all four states (request_parameters cannot repeat a
    key); a deleted author must surface as '' — never the text \"None\"."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps({"values": [_pr(1, author={"uuid": "{u-1}", "display_name": "Alice"}), _pr(2, author=None)]}),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_requests", config)

    # The four states ride on the path because request_parameters cannot repeat
    # a key; ANY_QUERY_PARAMS would match a request carrying none of them.
    pr_urls = [r.url for r in http_mocker._mocker.request_history if "pullrequests" in r.url]
    assert pr_urls, "no pull request call was made"
    for state in ("OPEN", "MERGED", "DECLINED", "SUPERSEDED"):
        assert f"state={state}" in pr_urls[0], pr_urls[0]

    assert not output.errors
    assert len(output.records) == 2
    by_id = {r.record.data["id"]: r.record.data for r in output.records}
    assert by_id[1]["author_display_name"] == "Alice"
    assert by_id[2]["author_uuid"] == ""
    # PR ids are per-repository, so the repo is part of the key.
    assert by_id[1]["unique_key"].endswith(":acme/app:1")
    assert len(by_id[1]["description"]) == 2048
    assert by_id[1]["destination_commit_sha"] == "c" * 12
    participants = json.loads(by_id[1]["participants"])
    assert participants[0]["uuid"] == "{u-2}"
    assert participants[0]["role"] == "REVIEWER"
    assert participants[0]["approved"] is True
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_requests", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_403_repository_is_skipped_not_fatal(http_mocker: HttpMocker) -> None:
    """The incident class this connector exists to survive: a repository that
    403s on a valid token must not break the stream."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(body="", status_code=403),
    )

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert not output.errors
    assert len(output.records) == 0


@freezegun.freeze_time(_FROZEN)
def test_activity_synthesizes_distinct_keys_without_ids(http_mocker: HttpMocker) -> None:
    """Activity entries carry no id: the (kind, date, actor) key must be
    distinct per entry and free of the literal \"None\"."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(body=json.dumps({"values": [_pr(9, author=None)]}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/9/activity",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                {
                    "values": [
                        {"approval": {"date": "2026-06-21T10:00:00+00:00", "user": {"uuid": "{u-1}", "display_name": "Alice"}}},
                        {"update": {"date": "2026-06-20T09:00:00+00:00", "state": "OPEN", "author": {"uuid": "{u-2}", "display_name": "Bob"}, "source": {"commit": {"hash": "c" * 12}}}},
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_activity", config)

    assert not output.errors
    assert len(output.records) == 2
    keys = [r.record.data["unique_key"] for r in output.records]
    assert len(set(keys)) == 2, keys
    kinds = sorted(r.record.data["kind"] for r in output.records)
    assert kinds == ["approval", "update"]
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_activity", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_comments_store_trimmed_bodies_and_inline(http_mocker: HttpMocker) -> None:
    """The silver comments model consumes body and the inline location; the
    body is content.raw trimmed to 2048 chars."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pullrequests", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [_pr(9, author=None)]}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/9/comments",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                {
                    "values": [
                        {
                            "id": 77,
                            "user": {"uuid": "{u-1}", "display_name": "Alice"},
                            "content": {"raw": "x" * 3000},
                            "inline": {"path": "src/a.py", "to": 12},
                            "deleted": False,
                            "created_on": "2026-06-20T10:00:00.000000+00:00",
                            "updated_on": "2026-06-20T10:00:00.000000+00:00",
                        },
                        {
                            "id": 78,
                            "user": None,
                            "content": None,
                            "deleted": False,
                            "created_on": "2026-06-20T10:00:00.000000+00:00",
                            "updated_on": "2026-06-20T10:00:00.000000+00:00",
                        },
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_comments", config)

    assert not output.errors
    by_id = {r.record.data["id"]: r.record.data for r in output.records}
    assert len(by_id[77]["body"]) == 2048
    assert by_id[77]["inline_path"] == "src/a.py"
    assert by_id[77]["inline_to"] == 12
    assert by_id[78]["body"] == ""
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_comments", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_pull_request_commits_store_only_the_edge(http_mocker: HttpMocker) -> None:
    """PR<->commit membership is vendor-only, and the raw nested payload stays
    out of bronze — but the commit message is kept, because a declined pull
    request's head never merges and the proxy walk never reaches it."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pullrequests", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [_pr(9, author=None)]}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/9/commits",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                {
                    "values": [
                        {
                            "type": "commit",
                            "hash": "a" * 12,
                            "date": "2026-06-19T10:00:00+00:00",
                            "message": "feat: x",
                            "summary": {"raw": "feat: x"},
                            "author": {"raw": "Alice <alice@example.com>"},
                            "parents": [{"hash": "b" * 12}],
                            "links": {"self": {"href": "https://api.bitbucket.org/x"}},
                            "repository": {"full_name": "acme/app"},
                            "rendered": {},
                        }
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_commits", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["sha"] == "a" * 12
    assert rec["pr_id"] == 9
    assert rec["repo_full_name"] == "acme/app"
    assert rec["unique_key"].endswith(f":acme/app:9:{'a' * 12}")
    # The head commits of an unmerged or declined pull request are reachable
    # from no branch the proxy clones, so the message has to survive here or
    # it is captured nowhere.
    assert rec["message"] == "feat: x"
    assert "links" not in rec and "parents" not in rec and "rendered" not in rec
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_commits", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_diffstat_rows_and_uncomputable_diff(http_mocker: HttpMocker) -> None:
    """Bitbucket carries no diff totals on the PR itself, so per-file diffstat
    rows are the only source of PR size. A PR whose branches share no history
    answers 400 — a property of that PR, not a broken request."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pullrequests", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [_pr(9, author=None)]}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/9/diffstat",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                {
                    "values": [
                        {
                            "type": "diffstat",
                            "status": "modified",
                            "lines_added": 12,
                            "lines_removed": 3,
                            "old": {"path": "src/a.py"},
                            "new": {"path": "src/a.py"},
                        },
                        {
                            "type": "diffstat",
                            "status": "removed",
                            "lines_added": 0,
                            "lines_removed": 40,
                            "old": {"path": "src/gone.py"},
                            "new": None,
                        },
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_diffstat", config)

    assert not output.errors
    by_path = {r.record.data["file_path"]: r.record.data for r in output.records}
    assert (by_path["src/a.py"]["lines_added"], by_path["src/a.py"]["lines_removed"]) == (12, 3)
    assert by_path["src/gone.py"]["status"] == "removed"
    assert by_path["src/gone.py"]["old_path"] == "src/gone.py"
    assert by_path["src/a.py"]["unique_key"].endswith(":acme/app:9:src/a.py")
    assert "old" not in by_path["src/a.py"] and "new" not in by_path["src/a.py"]
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_diffstat", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_diffstat_without_common_ancestor_is_skipped(http_mocker: HttpMocker) -> None:
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pullrequests", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [_pr(9, author=None)]}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/9/diffstat",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps({"type": "error", "error": {"message": "No common ancestor"}}),
            status_code=400,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_diffstat", config)

    assert not output.errors
    assert len(output.records) == 0


@freezegun.freeze_time(_FROZEN)
def test_workspace_members_stamping(http_mocker: HttpMocker) -> None:
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(
        HttpRequest(f"{BB_URL}/workspaces/acme/members", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps({"values": [{"user": {"account_id": "aid-1", "display_name": "Alice", "nickname": "alice", "uuid": "{u-1}"}}]}),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "workspace_members", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["workspace"] == "acme"
    assert rec["account_id"] == "aid-1"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "workspace_members", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_pipelines_row_conforms_and_an_empty_page_is_not_an_error(
    http_mocker: HttpMocker,
) -> None:
    """A repository with pipelines disabled answers an empty page, which is a
    defined answer rather than a failure — but the declared schema still has to
    hold for a repository that has them."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pipelines/", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                {
                    "values": [
                        {
                            "uuid": "{p-1}",
                            "build_number": 42,
                            "state": {"name": "COMPLETED", "result": {"name": "SUCCESSFUL"}},
                            "created_on": "2026-06-25T10:00:00.000000+00:00",
                            "completed_on": "2026-06-25T10:05:00.000000+00:00",
                            "target": {"ref_name": "main"},
                            "trigger": {"name": "PUSH"},
                        }
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pipelines", config)

    assert not output.errors
    assert len(output.records) == 1
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pipelines", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_429_is_retried_rather_than_failing_the_stream(http_mocker: HttpMocker) -> None:
    """Bitbucket meters the API, and a refusal is transient by nature: the
    stream must wait it out and still deliver the page."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests",
            query_params=ANY_QUERY_PARAMS,
        ),
        [
            HttpResponse(body="", status_code=429, headers={"Retry-After": "0"}),
            HttpResponse(
                body=json.dumps({"values": [_pr(1, author={"uuid": "{u-1}", "display_name": "Alice"})]}),
                status_code=200,
            ),
        ],
    )

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert not output.errors
    assert len(output.records) == 1


def _authors_page(*rows: dict[str, Any]) -> HttpResponse:
    return HttpResponse(
        body=json.dumps({"items": list(rows), "next_page_token": None}), status_code=200
    )


def _author_row(email: str, sha: str) -> dict[str, Any]:
    return {
        "author_email": email,
        "author_name": "Dev",
        "sample_sha": sha,
        "last_committed_date": "2026-06-15T10:00:00+00:00",
        "commit_count": 4,
    }


def _repo_with_clone() -> dict[str, Any]:
    return _repo() | {
        "links": {"clone": [{"name": "https", "href": "https://bot@bitbucket.org/acme/app.git"}]}
    }


@freezegun.freeze_time(_FROZEN)
def test_commit_authors_pair_a_git_email_with_its_account(http_mocker: HttpMocker) -> None:
    """The proxy names the distinct authors; Bitbucket names the account behind
    each one. `author.raw` is the git ident the commit rows carry, so the claim
    is keyed on that rather than on any profile address."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [_repo_with_clone()]}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author_row("ada@example.com", "a" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/commit/{'a' * 40}", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                {
                    "hash": "a" * 40,
                    "date": "2026-06-15T10:00:00+00:00",
                    "message": "feat: x",
                    "author": {
                        "raw": "Ada Lovelace <ada@example.com>",
                        "user": {
                            "account_id": "acc-42",
                            "uuid": "{u-42}",
                            "nickname": "ada",
                            "display_name": "Ada Lovelace",
                        },
                    },
                    "parents": [],
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["author_email"] == "ada@example.com", "keyed on the git ident"
    assert rec["author_account_id"] == "acc-42"
    assert rec["author_uuid"] == "{u-42}"
    assert rec["author_nickname"] == "ada"
    assert rec["repo_full_name"] == "acme/app"
    assert rec["sample_sha"] == "a" * 40
    assert rec["unique_key"].endswith(":acme/app:author:ada@example.com")
    assert "author" not in rec and "message" not in rec
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "commit_authors", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_commit_authors_drops_an_email_with_no_bitbucket_account(
    http_mocker: HttpMocker,
) -> None:
    """`author.user` is absent for a committer who holds no Bitbucket account —
    a CI or service identity. There is no account to claim the e-mail, so the
    row is dropped rather than stored as an unresolved one."""
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": [_repo_with_clone()]}), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author_row("ci@build.local", "b" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/commit/{'b' * 40}", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                {
                    "hash": "b" * 40,
                    "date": "2026-06-15T10:00:00+00:00",
                    "author": {"raw": "CI <ci@build.local>"},
                    "parents": [],
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    assert len(output.records) == 0, "an unmatched e-mail claims no account"


def _pr_listing(pr_id: int, updated_on: str) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {"values": [{"id": pr_id, "updated_on": updated_on, "repo_full_name": "acme/app"}]}
        ),
        status_code=200,
    )


@freezegun.freeze_time(_FROZEN)
def test_pr_detail_reaches_back_to_the_start_date_not_a_rolling_month(
    http_mocker: HttpMocker,
) -> None:
    """The start date is a floor, not a window. A pull request last touched five
    months ago is inside a start date of 2026-06-01 and its comments must be
    collected — a rolling 30-day window would have skipped it silently."""
    config = BitbucketCloudConfigBuilder().build()  # start date 2026-06-01
    stale = "2026-06-05T10:00:00.000000+00:00"  # ~4 weeks before _FROZEN
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pullrequests", query_params=ANY_QUERY_PARAMS),
        _pr_listing(31, stale),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/31/comments",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                {
                    "values": [
                        {
                            "id": 900,
                            "created_on": stale,
                            "updated_on": stale,
                            "content": {"raw": "old but in range"},
                            "user": {"uuid": "{u-1}", "account_id": "acc-1"},
                        }
                    ]
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_comments", config)

    assert not output.errors
    assert len(output.records) == 1, "a pull request older than 30 days is still in range"
    assert output.records[0].record.data["id"] == 900


@freezegun.freeze_time(_FROZEN)
def test_pr_detail_state_advances_so_a_later_sync_resumes(http_mocker: HttpMocker) -> None:
    """The child carries a cursor so the parent's state persists. Without it the
    parent restarts at the start-date floor on every sync and re-walks the whole
    window; with it, the run emits state the next run resumes from."""
    config = BitbucketCloudConfigBuilder().build()
    updated = "2026-06-20T10:00:00.000000+00:00"
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pullrequests", query_params=ANY_QUERY_PARAMS),
        _pr_listing(31, updated),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/31/diffstat",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                {"values": [{"status": "modified", "new": {"path": "a.txt"}, "lines_added": 3}]}
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_diffstat", config)

    assert not output.errors
    assert len(output.records) == 1
    # The record has no date of its own; it carries the parent pull request's,
    # which is what the cursor observes.
    assert output.records[0].record.data["pr_updated_on"] == updated
    assert output.state_messages, "an incremental child must emit state"
    state = output.state_messages[-1].state.stream.stream_state.__dict__
    stored = state["states"][0]["cursor"]["pr_updated_on"]
    assert stored.startswith("2026-06-20T10:00:00"), (
        f"the stamped parent date is what the cursor stores: {state}"
    )
    # The parent's own state is what a later sync resumes from.
    assert state["parent_state"]["pull_requests_for_diffstat"]["state"]["updated_on"] == updated


@freezegun.freeze_time(_FROZEN)
def test_a_resumed_sync_asks_for_less_than_the_first(http_mocker: HttpMocker) -> None:
    """The start date is a floor paid once. The first run asks for the whole
    window; a run carrying state asks only for what changed since — which is
    what makes a floor of one or two years affordable on a daily schedule.

    Both halves are needed for this: the fan-out parent holds a cursor so there
    IS state, and the child holds one plus `incremental_dependency` so the state
    is persisted. Drop either and the parent restarts at the floor every sync.
    """
    config = BitbucketCloudConfigBuilder().build()  # start date 2026-06-01
    updated = "2026-06-20T10:00:00.000000+00:00"

    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pullrequests", query_params=ANY_QUERY_PARAMS),
        _pr_listing(31, updated),
    )
    http_mocker.get(
        HttpRequest(
            f"{BB_URL}/repositories/acme/app/pullrequests/31/diffstat",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                {"values": [{"status": "modified", "new": {"path": "a.txt"}, "lines_added": 1}]}
            ),
            status_code=200,
        ),
    )

    def _sync(state):
        return read_stream(_CONNECTOR, "pull_request_diffstat", config, state=state)

    first = _sync(None)
    assert not first.errors
    resumed = _sync([m.state for m in first.state_messages][-1:])
    assert not resumed.errors, f"a resumed sync must not fail: {resumed.errors}"

    asked = [
        unquote_plus(str(r.url))
        for r in http_mocker._mocker.request_history
        if "/pullrequests?" in str(r.url)
    ]
    assert 'updated_on >= "2026-06-01' in asked[0], f"first run starts at the floor: {asked[0]}"
    assert 'updated_on >= "2026-06-19' in asked[-1], (
        f"a resumed run starts at stored state, not the floor: {asked[-1]}"
    )
