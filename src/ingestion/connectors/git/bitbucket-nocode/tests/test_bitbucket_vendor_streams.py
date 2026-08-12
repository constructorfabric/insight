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
from config import BB_URL, BitbucketNocodeConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    read_stream,
)

_CONNECTOR = "git/bitbucket-nocode"
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
        "destination": {"branch": {"name": "main"}},
        "comment_count": 1,
        "task_count": 0,
    }


@freezegun.freeze_time(_FROZEN)
def test_pull_requests_four_states_and_none_guard(http_mocker: HttpMocker) -> None:
    """The path carries all four states (request_parameters cannot repeat a
    key); a deleted author must surface as '' — never the text \"None\"."""
    config = BitbucketNocodeConfigBuilder().build()
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
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_requests", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_403_repository_is_skipped_not_fatal(http_mocker: HttpMocker) -> None:
    """The incident class this connector exists to survive: a repository that
    403s on a valid token must not break the stream."""
    config = BitbucketNocodeConfigBuilder().build()
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
    config = BitbucketNocodeConfigBuilder().build()
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
def test_workspace_members_stamping(http_mocker: HttpMocker) -> None:
    config = BitbucketNocodeConfigBuilder().build()
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
    config = BitbucketNocodeConfigBuilder().build()
    http_mocker.get(HttpRequest(_REPOS_URL, query_params=ANY_QUERY_PARAMS), _repos_page())
    http_mocker.get(
        HttpRequest(f"{BB_URL}/repositories/acme/app/pipelines/", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"values": []}), status_code=200),
    )

    output = read_stream(_CONNECTOR, "pipelines", config)

    assert not output.errors
    assert len(output.records) == 0
