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

import freezegun
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


def _no_literal_none(records) -> None:
    for r in records:
        for key, value in r.record.data.items():
            assert value != "None", f"literal 'None' leaked into {key}"


def _repo() -> dict:
    return {
        "uuid": "{r-1}",
        "full_name": "acme/app",
        "updated_on": "2026-06-20T10:00:00.000000+00:00",
    }


def _repos_page() -> HttpResponse:
    return HttpResponse(body=json.dumps({"values": [_repo()]}), status_code=200)


def _pr(pr_id: int, *, author: dict | None) -> dict:
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
    """PR<->commit membership is vendor-only; the commit payload belongs to
    the proxy streams, so bronze keeps the sha and the PR identity alone."""
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
    assert "message" not in rec and "links" not in rec
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


@freezegun.freeze_time(_FROZEN)
def test_pipelines_empty_page(http_mocker: HttpMocker) -> None:
    config = BitbucketCloudConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pipelines/", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": []}), status_code=200),
    )

    output = read_stream(_CONNECTOR, "pipelines", config)

    assert not output.errors
    assert len(output.records) == 0


def _authors_page(*rows: dict) -> HttpResponse:
    return HttpResponse(
        body=json.dumps({"items": list(rows), "next_page_token": None}), status_code=200
    )


def _author_row(email: str, sha: str) -> dict:
    return {
        "author_email": email,
        "author_name": "Dev",
        "sample_sha": sha,
        "last_committed_date": "2026-06-15T10:00:00+00:00",
        "commit_count": 4,
    }


def _repo_with_clone() -> dict:
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
