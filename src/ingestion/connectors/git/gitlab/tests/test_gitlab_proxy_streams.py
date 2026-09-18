"""Mock-server tests for the git-cli-proxy streams (commits, file_changes,
branches, and the author enumeration under commit_authors).

The proxy contract under test: cursor pagination on next_page_token, 429 +
Retry-After while a clone runs (retry, then succeed), 404/413 skipping the
project without failing the sync, and the project roster that decides which
clones happen at all — forks and excluded paths never reach the proxy, a
project idle since the start date is not cloned.
"""

from __future__ import annotations

import json
from typing import Any

import freezegun
from airbyte_cdk.models import FailureType
from config import API_URL, GITLAB_URL, PROXY_URL, GitlabConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, assert_records_conform, read_stream

_CONNECTOR = "git/gitlab"
_PROJECTS_URL = f"{API_URL}/groups/acme/projects"
_CLONE_URL = f"{GITLAB_URL}/acme/app.git"
_FROZEN = "2026-07-01T00:00:00Z"


def _project(**overrides: Any) -> dict[str, Any]:
    return {
        "id": 7,
        "path": "app",
        "path_with_namespace": "acme/app",
        "http_url_to_repo": _CLONE_URL,
        "default_branch": "main",
        "archived": False,
        "statistics": {"repository_size": 734003200},
        "last_activity_at": "2026-06-20T10:00:00.000+00:00",
        **overrides,
    }


def _projects_page(*projects: dict[str, Any]) -> HttpResponse:
    return HttpResponse(body=json.dumps(list(projects or [_project()])), status_code=200)


def _commit(sha: str, committed: str = "2026-06-15T10:00:00Z") -> dict[str, Any]:
    return {
        "sha": sha,
        "message": f"commit {sha}",
        "authored_date": committed,
        "committed_date": committed,
        "author_name": "Dev",
        "author_email": "dev@example.com",
        "committer_name": "Dev",
        "committer_email": "dev@example.com",
        "parent_hashes": [],
        "is_merge": False,
        "is_in_default_branch": True,
        "patch_id": None,
    }


def _page(items: list[dict[str, Any]], next_token: str | None = None) -> HttpResponse:
    return HttpResponse(body=json.dumps({"items": items, "next_page_token": next_token}), status_code=200)


def _proxy_calls(http_mocker: HttpMocker, endpoint: str) -> list[str]:
    return [r.url for r in http_mocker._mocker.request_history if f"/v1/{endpoint}" in r.url]


@freezegun.freeze_time(_FROZEN)
def test_commits_paginate_and_key_on_the_project_id(http_mocker: HttpMocker) -> None:
    """Forks share SHAs, so the project is part of the key — by numeric id, which
    survives a rename, not by path."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS),
        [_page([_commit("a" * 40)], next_token="t1"), _page([_commit("b" * 40)])],
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    assert len(output.records) == 2
    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["data_source"] == "insight_gitlab"
    assert rec["project_id"] == 7
    assert rec["repo_path"] == "acme/app"
    assert rec["repository"] == _CLONE_URL
    assert rec["unique_key"] == f"test-tenant:test-source:7:{'a' * 40}"
    calls = _proxy_calls(http_mocker, "commits")
    assert "page_token=t1" in calls[1]
    assert "since=2026-06-01" in calls[0], "the start date floors the walk"
    hints = {r.headers.get("X-Repo-Size-Hint") for r in http_mocker._mocker.request_history if "/v1/commits" in r.url}
    assert hints == {"734003200"}, f"every proxy call carries the project's reported size: {hints}"
    assert_records_conform(output.records, _CONNECTOR, "commits", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_resumed_commits_sync_clones_no_project_idle_since_the_stored_cursor(http_mocker: HttpMocker) -> None:
    """The roster's last_activity_at cursor is persisted with the child
    (incremental_dependency), so a later run never clones a project whose
    activity predates what the previous run already saw."""
    config = GitlabConfigBuilder().build()
    idle = _project(
        id=8,
        path="api",
        path_with_namespace="acme/api",
        http_url_to_repo=f"{GITLAB_URL}/acme/api.git",
        last_activity_at="2026-06-10T10:00:00.000+00:00",
    )
    http_mocker.get(
        HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), [_projects_page(), _projects_page(_project(), idle)]
    )
    http_mocker.get(HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS), _page([_commit("a" * 40)]))

    first = read_stream(_CONNECTOR, "commits", config)
    assert not first.errors
    assert first.state_messages, "an incremental child must emit state"
    state = first.state_messages[-1].state.stream.stream_state.__dict__
    assert state["parent_state"]["projects_active"]["states"], f"the roster cursor must be persisted: {state}"
    resumed = read_stream(_CONNECTOR, "commits", config, state=[m.state for m in first.state_messages][-1:])
    assert not resumed.errors, f"a resumed sync must not fail: {resumed.errors}"

    cloned = _proxy_calls(http_mocker, "commits")
    assert not any("acme%2Fapi.git" in url or "acme/api.git" in url for url in cloned), cloned


@freezegun.freeze_time(_FROZEN)
def test_a_resumed_sync_re_admits_a_project_active_inside_the_day_before_the_cursor(http_mocker: HttpMocker) -> None:
    """GitLab moves last_activity_at at most once an hour, so a push soon
    after the previous sync can leave its project below the stored cursor;
    the roster re-lists a day back so the project is still walked."""
    config = GitlabConfigBuilder().build()
    recent = _project(
        id=8,
        path="api",
        path_with_namespace="acme/api",
        http_url_to_repo=f"{GITLAB_URL}/acme/api.git",
        last_activity_at="2026-06-19T20:00:00.000+00:00",
    )
    http_mocker.get(
        HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS),
        [_projects_page(), _projects_page(_project(), recent)],
    )
    http_mocker.get(HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS), _page([_commit("a" * 40)]))

    first = read_stream(_CONNECTOR, "commits", config)
    assert not first.errors
    resumed = read_stream(_CONNECTOR, "commits", config, state=[m.state for m in first.state_messages][-1:])
    assert not resumed.errors, f"a resumed sync must not fail: {resumed.errors}"

    cloned = _proxy_calls(http_mocker, "commits")
    assert any("acme%2Fapi.git" in url or "acme/api.git" in url for url in cloned), cloned


@freezegun.freeze_time(_FROZEN)
def test_a_private_commit_address_names_its_account_only_on_this_instance(http_mocker: HttpMocker) -> None:
    """`{id}-{username}@users.noreply.<host>` states the account id, but the
    same form minted by another GitLab names an account there, not here."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    ours = {**_commit("a" * 40), "author_email": "42-Ada@users.noreply.gitlab.example.com"}
    foreign = {**_commit("b" * 40), "author_email": "42-ada@users.noreply.gitlab.com"}
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS), _page([ours, foreign, _commit("c" * 40)])
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    by_sha = {r.record.data["sha"]: r.record.data.get("author_account_id") for r in output.records}
    assert by_sha == {"a" * 40: 42, "b" * 40: None, "c" * 40: None}, by_sha
    assert_records_conform(output.records, _CONNECTOR, "commits", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_changed_commit_email_hostname_is_the_one_matched(http_mocker: HttpMocker) -> None:
    """An administrator can move the private address form to another host;
    then that host names accounts and the default form no longer does."""
    config = {**GitlabConfigBuilder().build(), "gitlab_commit_email_hostname": "noreply.corp.example"}
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    moved = {**_commit("a" * 40), "author_email": "9-bob@noreply.corp.example"}
    default_form = {**_commit("b" * 40), "author_email": "42-ada@users.noreply.gitlab.example.com"}
    http_mocker.get(HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS), _page([moved, default_form]))

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    by_sha = {r.record.data["sha"]: r.record.data.get("author_account_id") for r in output.records}
    assert by_sha == {"a" * 40: 9, "b" * 40: None}, by_sha


@freezegun.freeze_time(_FROZEN)
def test_a_proxy_401_is_the_proxy_token_and_fails_as_a_config_error(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/branches", query_params=ANY_QUERY_PARAMS), HttpResponse(body="", status_code=401)
    )

    output = read_stream(_CONNECTOR, "branches", config, expecting_exception=True)

    assert output.errors
    assert output.errors[-1].trace.error.failure_type == FailureType.config_error


@freezegun.freeze_time(_FROZEN)
def test_429_while_the_clone_runs_is_a_wait_not_a_failure(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS),
        [HttpResponse(body="", status_code=429, headers={"Retry-After": "0"}), _page([_commit("c" * 40)])],
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    assert len(output.records) == 1


@freezegun.freeze_time(_FROZEN)
def test_a_project_gone_at_origin_is_skipped_not_fatal(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS), HttpResponse(body="", status_code=404)
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    assert output.records == []


@freezegun.freeze_time(_FROZEN)
def test_file_changes_never_ask_for_the_patch(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/file-changes", query_params=ANY_QUERY_PARAMS),
        _page(
            [
                {
                    "sha": "a" * 40,
                    "committed_date": "2026-06-15T10:00:00Z",
                    "filename": "src/main.rs",
                    "status": "modified",
                    "additions": 3,
                    "deletions": 1,
                    "pre_image_oid": "1" * 40,
                    "post_image_oid": "2" * 40,
                }
            ]
        ),
    )

    output = read_stream(_CONNECTOR, "file_changes", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["unique_key"] == f"test-tenant:test-source:7:{'a' * 40}:src/main.rs"
    assert "include_patch=false" in _proxy_calls(http_mocker, "file-changes")[0]
    assert_records_conform(output.records, _CONNECTOR, "file_changes", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_branches_key_on_project_and_name(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/branches", query_params=ANY_QUERY_PARAMS),
        _page(
            [{"name": "main", "head_sha": "f" * 40, "head_committed_date": "2026-06-20T10:00:00Z", "is_default": True}]
        ),
    )

    output = read_stream(_CONNECTOR, "branches", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["unique_key"] == "test-tenant:test-source:7:main"
    assert rec["is_default"] is True
    assert_records_conform(output.records, _CONNECTOR, "branches", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_roster_admits_neither_forks_nor_excluded_paths_nor_idle_projects(http_mocker: HttpMocker) -> None:
    """A fork carries its upstream's whole history, an excluded path is the
    operator's call, and a project idle since the start date holds nothing
    inside the window: none of the three is worth a clone."""
    config = GitlabConfigBuilder().with_field("gitlab_exclude_projects", ["^acme/sandbox/"]).build()
    fork = _project(
        id=8,
        path_with_namespace="acme/app-fork",
        http_url_to_repo=f"{GITLAB_URL}/acme/app-fork.git",
        forked_from_project={"id": 7, "path_with_namespace": "acme/app"},
    )
    excluded = _project(
        id=9, path_with_namespace="acme/sandbox/scratch", http_url_to_repo=f"{GITLAB_URL}/acme/sandbox/scratch.git"
    )
    idle = _project(
        id=10,
        path_with_namespace="acme/dormant",
        http_url_to_repo=f"{GITLAB_URL}/acme/dormant.git",
        last_activity_at="2024-01-01T00:00:00.000+00:00",
    )
    http_mocker.get(
        HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page(_project(), fork, excluded, idle)
    )
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/branches", query_params=ANY_QUERY_PARAMS),
        _page(
            [{"name": "main", "head_sha": "f" * 40, "head_committed_date": "2026-06-20T10:00:00Z", "is_default": True}]
        ),
    )

    output = read_stream(_CONNECTOR, "branches", config)

    assert not output.errors
    walked = {r.record.data["repository"] for r in output.records}
    assert walked == {_CLONE_URL}, walked


@freezegun.freeze_time(_FROZEN)
def test_forks_are_admitted_when_the_operator_asks(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().with_field("gitlab_include_forks", "true").build()
    fork = _project(
        id=8,
        path_with_namespace="acme/app-fork",
        http_url_to_repo=f"{GITLAB_URL}/acme/app-fork.git",
        forked_from_project={"id": 7, "path_with_namespace": "acme/app"},
    )
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page(_project(), fork))
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/branches", query_params=ANY_QUERY_PARAMS),
        _page(
            [{"name": "main", "head_sha": "f" * 40, "head_committed_date": "2026-06-20T10:00:00Z", "is_default": True}]
        ),
    )

    output = read_stream(_CONNECTOR, "branches", config)

    assert {r.record.data["project_id"] for r in output.records} == {7, 8}


def _authors_page(*rows: dict[str, Any]) -> HttpResponse:
    return HttpResponse(body=json.dumps({"items": list(rows), "next_page_token": None}), status_code=200)


def _author(email: str, sha: str, committed: str = "2026-06-15T10:00:00+00:00") -> dict[str, Any]:
    return {
        "author_email": email,
        "author_name": "Ada",
        "sample_sha": sha,
        "last_committed_date": committed,
        "commit_count": 3,
    }


def _user(uid: int, username: str, **fields: Any) -> dict[str, Any]:
    return {
        "id": uid,
        "username": username,
        "name": username.title(),
        "state": "active",
        "avatar_url": "x",
        "web_url": "y",
        **fields,
    }


@freezegun.freeze_time(_FROZEN)
def test_commit_authors_claim_an_account_only_on_an_exact_address_match(http_mocker: HttpMocker) -> None:
    """`/users?search=` matches on name and username too; a hit whose address
    is not the git ident claims nothing. Case differs between a git ident and
    a profile, so the comparison folds it."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author("Ada@example.com", "a" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/users", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    _user(41, "ada.other", public_email="ada.other@example.com"),
                    _user(42, "ada", public_email="ada@example.com"),
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["author_email"] == "Ada@example.com", "keyed on the git ident, not the profile"
    assert (rec["author_account_id"], rec["author_username"], rec["matched_field"]) == (42, "ada", "public_email")
    assert rec["project_id"] == 7
    assert rec["sample_sha"] == "a" * 40
    assert rec["unique_key"] == "test-tenant:test-source:7:ada@example.com", (
        "the key folds case; the column keeps the ident"
    )
    assert "avatar_url" not in rec
    lookup = next(r.url for r in http_mocker._mocker.request_history if "/users" in r.url)
    assert "search=Ada%40example.com" in lookup
    assert_records_conform(output.records, _CONNECTOR, "commit_authors", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_resumed_commit_authors_sync_asks_each_project_only_for_newer_authors(http_mocker: HttpMocker) -> None:
    """The author walk is a child of the roster twice over; both cursors ride
    along with the stream's state, so a later run asks the proxy for authors
    who committed since the last one instead of since the start date."""
    config = {**GitlabConfigBuilder().build(), "gitlab_start_date": "2026-03-01"}
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author("ada@example.com", "a" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/users", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_user(42, "ada", public_email="ada@example.com")]), status_code=200),
    )

    first = read_stream(_CONNECTOR, "commit_authors", config)
    assert not first.errors
    assert first.state_messages, "an incremental child must emit state"
    state = first.state_messages[-1].state.stream.stream_state.__dict__
    assert state["parent_state"]["repository_authors"]["states"], f"the author walk's cursor must be persisted: {state}"
    resumed = read_stream(_CONNECTOR, "commit_authors", config, state=[m.state for m in first.state_messages][-1:])
    assert not resumed.errors, f"a resumed sync must not fail: {resumed.errors}"

    asked = _proxy_calls(http_mocker, "authors")
    assert "since=2026-03-01" in asked[0], f"first run starts at the floor: {asked[0]}"
    assert "since=2026-05-15" in asked[-1], f"a resumed run starts one window before the stored cursor: {asked[-1]}"


@freezegun.freeze_time(_FROZEN)
def test_a_future_dated_commit_never_becomes_the_commits_cursor(http_mocker: HttpMocker) -> None:
    """A committer clock set ahead would otherwise become the saved cursor and
    every later sync would ask for commits since a date that has not come."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/commits", query_params=ANY_QUERY_PARAMS),
        _page([_commit("a" * 40), _commit("b" * 40, committed="2099-01-01T00:00:00Z")]),
    )

    output = read_stream(_CONNECTOR, "commits", config)

    assert not output.errors
    assert [r.record.data["sha"] for r in output.records] == ["a" * 40]
    saved = json.dumps(output.state_messages[-1].state.stream.stream_state.__dict__)
    assert "2099" not in saved, f"the future date leaked into state: {saved}"
    assert "2026-06-15T10:00:00" in saved, f"the newest real date must be the cursor: {saved}"


@freezegun.freeze_time(_FROZEN)
def test_a_future_dated_author_never_becomes_the_authors_since(http_mocker: HttpMocker) -> None:
    """An author whose last commit is dated ahead would push the author list's
    `since` past now and the list would come back empty on every later sync."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(
            _author("ada@example.com", "a" * 40),
            _author("zed@example.com", "b" * 40, committed="2099-01-01T00:00:00+00:00"),
        ),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/users", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([_user(42, "ada", public_email="ada@example.com")]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    assert [r.record.data["author_email"] for r in output.records] == ["ada@example.com"]
    saved = json.dumps(output.state_messages[-1].state.stream.stream_state.__dict__)
    assert "2099" not in saved, f"the future date leaked into state: {saved}"


@freezegun.freeze_time(_FROZEN)
def test_commit_authors_drop_an_address_no_account_carries(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _projects_page())
    http_mocker.get(
        HttpRequest(f"{PROXY_URL}/v1/authors", query_params=ANY_QUERY_PARAMS),
        _authors_page(_author("ci@build.local", "b" * 40)),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/users", query_params=ANY_QUERY_PARAMS), HttpResponse(body="[]", status_code=200)
    )

    output = read_stream(_CONNECTOR, "commit_authors", config)

    assert not output.errors
    assert output.records == []
