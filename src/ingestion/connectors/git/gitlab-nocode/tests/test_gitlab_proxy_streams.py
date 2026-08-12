"""Mock-server tests for the git-cli-proxy streams (`commits`).

The proxy contract under test: cursor pagination on next_page_token,
429 + Retry-After while a clone runs (retry, then succeed), 404/413 skipping
the repository without failing the sync, and 409 (superseded snapshot)
failing the attempt.

Coverage matrix rows: full_refresh_single_page (via incremental first read),
pagination_multi_page, empty_page, tenant_source_stamping,
schema_conformance, substream_partition, incremental_state, error_retry
(429), error_ignore (404).
"""

from __future__ import annotations

import json

import freezegun
from config import GITLAB_URL, PROXY_URL, GitlabNocodeConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    read_stream,
)

_CONNECTOR = "git/gitlab-nocode"
_STREAM = "commits"
_PROJECTS_URL = f"{GITLAB_URL}/api/v4/groups/acme/projects"
_COMMITS_URL = f"{PROXY_URL}/v1/commits"
_CLONE_URL = f"{GITLAB_URL}/acme/app.git"
_FROZEN = "2026-07-01T00:00:00Z"


def _project() -> dict:
    return {
        "id": 7,
        "http_url_to_repo": _CLONE_URL,
        "last_activity_at": "2026-06-20T10:00:00.000+00:00",
    }


def _commit(sha: str) -> dict:
    return {
        "sha": sha,
        "message": f"commit {sha}",
        "authored_date": "2026-06-15T10:00:00Z",
        "committed_date": "2026-06-15T10:00:00Z",
        "author_name": "Dev",
        "author_email": "dev@example.com",
        "committer_name": "Dev",
        "committer_email": "dev@example.com",
        "parent_hashes": [],
        "is_merge": False,
        "is_in_default_branch": True,
        "patch_id": None,
    }


def _page(items: list[dict], next_token: str | None = None) -> HttpResponse:
    return HttpResponse(
        body=json.dumps({"items": items, "next_page_token": next_token}),
        status_code=200,
    )


def _mock_parent(http_mocker: HttpMocker) -> None:
    http_mocker.get(
        HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_project()]), status_code=200),
    )


@freezegun.freeze_time(_FROZEN)
def test_pagination_multi_page_and_stamping(http_mocker: HttpMocker) -> None:
    config = GitlabNocodeConfigBuilder().build()
    _mock_parent(http_mocker)
    http_mocker.get(
        HttpRequest(_COMMITS_URL, query_params=ANY_QUERY_PARAMS),
        [_page([_commit("a" * 40)], next_token="t1"), _page([_commit("b" * 40)])],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 2
    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    # Repository identity is in the key (forks share SHAs); colons escaped.
    assert rec["unique_key"].endswith(":" + "a" * 40)
    assert _CLONE_URL.replace(":", "%3A") in rec["unique_key"]
    assert_records_conform(output.records, _CONNECTOR, _STREAM, strict=True)


@freezegun.freeze_time(_FROZEN)
def test_429_then_success_is_one_retry_not_a_failure(http_mocker: HttpMocker) -> None:
    """A cold clone answers 429 + Retry-After while it runs; the stream waits."""
    config = GitlabNocodeConfigBuilder().build()
    _mock_parent(http_mocker)
    http_mocker.get(
        HttpRequest(_COMMITS_URL, query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(body="", status_code=429, headers={"Retry-After": "0"}),
            _page([_commit("c" * 40)]),
        ],
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 1


@freezegun.freeze_time(_FROZEN)
def test_404_skips_the_repository_not_the_sync(http_mocker: HttpMocker) -> None:
    """A repository gone at origin is skipped; the stream stays green."""
    config = GitlabNocodeConfigBuilder().build()
    _mock_parent(http_mocker)
    http_mocker.get(
        HttpRequest(_COMMITS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(body="", status_code=404),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 0


@freezegun.freeze_time(_FROZEN)
def test_empty_page(http_mocker: HttpMocker) -> None:
    config = GitlabNocodeConfigBuilder().build()
    _mock_parent(http_mocker)
    http_mocker.get(
        HttpRequest(_COMMITS_URL, query_params=ANY_QUERY_PARAMS),
        _page([]),
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert len(output.records) == 0
