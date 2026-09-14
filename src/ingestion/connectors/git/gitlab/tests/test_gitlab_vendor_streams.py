"""Mock-server tests for the GitLab API streams.

The vendor hazards under test: the three scope modes and the endpoint family
each selects, the None-guard on every hoisted field (a deleted account leaves
`author` null; the column must read '' — never the text "None"), the
`draft`/`work_in_progress` and `merge_user`/`merged_by` renames across GitLab
versions, per-project 403/404 skipping against configured-scope 403/404
failing loudly, GraphQL errors inside an HTTP 200, and the deployment key
that keeps one row per status.
"""

from __future__ import annotations

import json
from typing import Any

import freezegun
import pytest
from airbyte_cdk.models import FailureType
from config import API_URL, GRAPHQL_URL, GitlabConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, assert_records_conform, read_stream
from connector_tests.source import load_manifest

_CONNECTOR = "git/gitlab"
_PROJECTS_URL = f"{API_URL}/groups/acme/projects"
_MRS_URL = f"{API_URL}/groups/acme/merge_requests"
_FROZEN = "2026-07-01T00:00:00Z"


def _no_literal_none(records) -> None:
    for r in records:
        for key, value in r.record.data.items():
            assert value != "None", f"literal 'None' leaked into {key}"


def _ok(payload: Any) -> HttpResponse:
    return HttpResponse(body=json.dumps(payload), status_code=200)


def _project(**overrides: Any) -> dict[str, Any]:
    return {
        "id": 7,
        "name": "app",
        "path": "app",
        "path_with_namespace": "acme/app",
        "namespace": {"id": 3, "full_path": "acme", "kind": "group"},
        "http_url_to_repo": "https://gitlab.example.com/acme/app.git",
        "web_url": "https://gitlab.example.com/acme/app",
        "default_branch": "main",
        "visibility": "private",
        "archived": False,
        "empty_repo": False,
        "forked_from_project": None,
        "topics": ["rust"],
        "statistics": {"repository_size": 1024},
        "created_at": "2020-01-01T00:00:00.000+00:00",
        "last_activity_at": "2026-06-20T10:00:00.000+00:00",
        **overrides,
    }


def _mr(iid: int, **overrides: Any) -> dict[str, Any]:
    return {
        "id": 1000 + iid,
        "iid": iid,
        "project_id": 7,
        "state": "merged",
        "draft": False,
        "title": f"MR {iid}",
        "description": "d" * 3000,
        "author": {"id": 11, "username": "alice", "name": "Alice"},
        "merge_user": {"id": 12, "username": "bob", "name": "Bob"},
        "merged_by": {"id": 12, "username": "bob", "name": "Bob"},
        "closed_by": None,
        "assignees": [{"id": 11, "username": "alice"}],
        "reviewers": [{"id": 12, "username": "bob", "name": "Bob"}],
        "labels": ["backend"],
        "milestone": {"id": 5, "title": "v1"},
        "source_branch": "feat",
        "target_branch": "main",
        "sha": "e" * 40,
        "merge_commit_sha": "f" * 40,
        "changes_count": "3",
        "created_at": "2026-06-10T10:00:00.000+00:00",
        "updated_at": "2026-06-20T10:00:00.000+00:00",
        "merged_at": "2026-06-20T10:00:00.000+00:00",
        "closed_at": None,
        "user_notes_count": 2,
        **overrides,
    }


def _urls(http_mocker: HttpMocker, fragment: str) -> list[str]:
    return [r.url for r in http_mocker._mocker.request_history if fragment in r.url]


# ── scope modes ──────────────────────────────────────────────────────────


@freezegun.freeze_time(_FROZEN)
def test_repositories_hoist_the_namespace_and_key_on_the_project_id(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _ok([_project()]))

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["unique_key"] == "test-tenant:test-source:7"
    assert (rec["namespace_full_path"], rec["namespace_kind"], rec["namespace_id"]) == ("acme", "group", 3)
    assert rec["is_fork"] is False
    assert rec["repository_size"] == 1024
    assert json.loads(rec["topics"]) == ["rust"]
    assert "namespace" not in rec and "statistics" not in rec
    listing = _urls(http_mocker, "/groups/acme/projects")[0]
    assert "include_subgroups=true" in listing and "with_shared=false" in listing and "archived=false" in listing
    assert "order_by=id" in listing and "sort=asc" in listing, f"pages must follow an immutable key: {listing}"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "repositories", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_configured_project_is_read_as_a_single_object(http_mocker: HttpMocker) -> None:
    """`/projects/:path` answers one object, not a list; the group-only
    parameters must not ride along."""
    config = (
        GitlabConfigBuilder().with_field("gitlab_groups", []).with_field("gitlab_projects", ["acme/tools/cli"]).build()
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/acme%2Ftools%2Fcli", query_params=ANY_QUERY_PARAMS),
        _ok(_project(id=21, path_with_namespace="acme/tools/cli")),
    )

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [21]
    url = _urls(http_mocker, "/projects/acme%2Ftools%2Fcli")[0]
    assert "include_subgroups" not in url and "order_by" not in url and "sort" not in url


def test_an_archived_configured_project_yields_no_roster_row(http_mocker: HttpMocker) -> None:
    """The single-object endpoint has no `archived` filter, so the roster
    applies the rule the group listing gets from its query parameter."""
    config = (
        GitlabConfigBuilder().with_field("gitlab_groups", []).with_field("gitlab_projects", ["acme/tools/cli"]).build()
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/acme%2Ftools%2Fcli", query_params=ANY_QUERY_PARAMS),
        _ok(_project(id=21, path_with_namespace="acme/tools/cli", archived=True)),
    )

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    assert output.records == []


@freezegun.freeze_time(_FROZEN)
def test_nothing_configured_walks_the_instance_with_keyset_pagination(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().with_field("gitlab_groups", []).build()
    http_mocker.get(HttpRequest(f"{API_URL}/projects", query_params=ANY_QUERY_PARAMS), _ok([_project()]))

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    url = _urls(http_mocker, "/projects?")[0]
    assert "pagination=keyset" in url and "order_by=id" in url and "sort=asc" in url
    assert "include_subgroups" not in url


@freezegun.freeze_time(_FROZEN)
def test_a_group_the_token_cannot_see_fails_as_a_config_error(http_mocker: HttpMocker) -> None:
    """Scope discovery is the operator's configuration; a 404 there is not a
    project to skip but a wrong path or a token without membership."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), HttpResponse(body="", status_code=404))

    output = read_stream(_CONNECTOR, "repositories", config, expecting_exception=True)

    assert output.errors
    assert output.errors[-1].trace.error.failure_type == FailureType.config_error


# ── merge requests ───────────────────────────────────────────────────────


@freezegun.freeze_time(_FROZEN)
def test_pull_requests_hoist_people_and_survive_a_deleted_account(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS),
        _ok([_mr(1), _mr(2, author=None, merge_user=None, merged_by=None, milestone=None, reviewers=[], labels=[])]),
    )

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert not output.errors
    by_iid = {r.record.data["iid"]: r.record.data for r in output.records}
    first = by_iid[1]
    assert (first["author_id"], first["author_username"], first["author_name"]) == (11, "alice", "Alice")
    assert (first["merged_by_id"], first["merged_by_username"]) == (12, "bob")
    assert json.loads(first["reviewers"])[0]["username"] == "bob"
    assert json.loads(first["assignee_ids"]) == [11]
    assert json.loads(first["labels"]) == ["backend"]
    assert (first["milestone_id"], first["milestone_title"]) == (5, "v1")
    assert len(first["description"]) == 2048
    assert first["unique_key"] == "test-tenant:test-source:7:1"
    assert "author" not in first and "merge_user" not in first and "merged_by" not in first
    second = by_iid[2]
    assert second["author_username"] == "" and second["merged_by_username"] == ""
    assert second.get("author_id") is None
    assert second["milestone_title"] == ""
    listing = _urls(http_mocker, "/merge_requests")[0]
    assert "scope=all" in listing and "state=all" in listing and "updated_after=2026-06-01" in listing
    assert "order_by=created_at" in listing and "sort=asc" in listing, f"pages must follow an immutable key: {listing}"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_requests", strict=True)


@pytest.mark.parametrize(
    ("payload", "expected"),
    [
        ({"draft": True}, True),
        ({"draft": None, "work_in_progress": True}, True),
        ({"draft": None, "work_in_progress": None}, False),
    ],
)
@freezegun.freeze_time(_FROZEN)
def test_draft_falls_back_to_work_in_progress_on_older_instances(
    http_mocker: HttpMocker, payload: dict[str, Any], expected: bool
) -> None:
    config = GitlabConfigBuilder().build()
    mr = _mr(1)
    mr.pop("draft")
    mr.update(payload)
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([mr]))

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["draft"] is expected, f"should read draft={expected}: {payload}"
    assert "work_in_progress" not in rec


@freezegun.freeze_time(_FROZEN)
def test_merged_by_is_read_when_merge_user_is_absent(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    mr = _mr(1)
    mr.pop("merge_user")
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([mr]))

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert output.records[0].record.data["merged_by_username"] == "bob"


@freezegun.freeze_time(_FROZEN)
def test_notes_keep_system_notes_and_inline_positions(http_mocker: HttpMocker) -> None:
    """System notes carry the review verdicts; user notes are the comments.
    Both land in one stream and the staging models split them."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([_mr(5)]))
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/merge_requests/5/notes", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 501,
                    "type": "DiffNote",
                    "body": "n" * 3000,
                    "system": False,
                    "resolvable": True,
                    "resolved": False,
                    "author": {"id": 11, "username": "alice", "name": "Alice"},
                    "position": {"new_path": "src/a.py", "old_path": "src/a.py", "new_line": 3, "old_line": None},
                    "created_at": "2026-06-20T10:00:00.000+00:00",
                    "updated_at": "2026-06-20T10:00:00.000+00:00",
                },
                {
                    "id": 502,
                    "type": None,
                    "body": "approved this merge request",
                    "system": True,
                    "author": {"id": 12, "username": "bob", "name": "Bob"},
                    "created_at": "2026-06-20T11:00:00.000+00:00",
                    "updated_at": "2026-06-20T11:00:00.000+00:00",
                },
            ]
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_notes", config)

    assert not output.errors
    by_id = {r.record.data["id"]: r.record.data for r in output.records}
    assert len(by_id[501]["body"]) == 2048
    assert (by_id[501]["author_id"], by_id[501]["position_new_path"], by_id[501]["position_new_line"]) == (
        11,
        "src/a.py",
        3,
    )
    assert by_id[501]["type"] == "DiffNote"
    assert by_id[502]["system"] is True and by_id[502]["type"] == ""
    assert by_id[502]["position_new_path"] == ""
    assert by_id[502]["mr_iid"] == 5 and by_id[502]["project_id"] == 7
    assert by_id[502]["mr_updated_at"] == "2026-06-20T10:00:00.000+00:00"
    assert by_id[502]["unique_key"] == "test-tenant:test-source:7:5:502"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_notes", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_merge_request_commits_carry_the_commit_and_the_edge(http_mocker: HttpMocker) -> None:
    """A squash-merged request's commits reach no ref the proxy clones, so the
    commit metadata travels with the membership edge."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([_mr(5)]))
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/merge_requests/5/commits", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": "a" * 40,
                    "short_id": "a" * 8,
                    "title": "feat: x",
                    "message": "feat: x\n",
                    "author_name": "Alice",
                    "author_email": "alice@example.com",
                    "authored_date": "2026-06-19T10:00:00.000+00:00",
                    "committer_name": "Alice",
                    "committer_email": "alice@example.com",
                    "committed_date": "2026-06-19T10:00:00.000+00:00",
                    "created_at": "2026-06-19T10:00:00.000+00:00",
                    "parent_ids": ["b" * 40],
                    "trailers": {},
                    "web_url": "https://gitlab.example.com/acme/app/-/commit/" + "a" * 40,
                }
            ]
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_commits", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["sha"] == "a" * 40 and "id" not in rec
    assert (rec["mr_iid"], rec["project_id"], rec["author_email"]) == (5, 7, "alice@example.com")
    assert json.loads(rec["parent_ids"]) == ["b" * 40]
    assert rec["unique_key"] == f"test-tenant:test-source:7:5:{'a' * 40}"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_commits", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_state_and_label_events_keep_the_actor_and_distinct_keys(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([_mr(5)]))
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/merge_requests/5/resource_state_events", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 9,
                    "user": {"id": 12, "username": "bob", "name": "Bob"},
                    "state": "merged",
                    "resource_type": "MergeRequest",
                    "resource_id": 1005,
                    "created_at": "2026-06-20T10:00:00.000+00:00",
                }
            ]
        ),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/merge_requests/5/resource_label_events", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 9,
                    "user": None,
                    "action": "add",
                    "label": {"id": 3, "name": "backend"},
                    "resource_type": "MergeRequest",
                    "resource_id": 1005,
                    "created_at": "2026-06-11T10:00:00.000+00:00",
                }
            ]
        ),
    )

    states = read_stream(_CONNECTOR, "pull_request_state_events", config)
    labels = read_stream(_CONNECTOR, "pull_request_label_events", config)

    assert not states.errors and not labels.errors
    state = states.records[0].record.data
    label = labels.records[0].record.data
    assert (state["state"], state["user_username"]) == ("merged", "bob")
    assert (label["action"], label["label_name"], label["user_username"]) == ("add", "backend", "")
    assert state["unique_key"] == "test-tenant:test-source:7:5:state:9"
    assert label["unique_key"] == "test-tenant:test-source:7:5:label:9"
    _no_literal_none(states.records)
    _no_literal_none(labels.records)
    assert_records_conform(states.records, _CONNECTOR, "pull_request_state_events", strict=True)
    assert_records_conform(labels.records, _CONNECTOR, "pull_request_label_events", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_merge_requests_sharing_an_iid_across_projects_each_get_their_children(http_mocker: HttpMocker) -> None:
    """The iid is numbered per project, so two projects' fifth merge requests
    must fan out as two partitions, not collapse into one."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([_mr(5), _mr(5, id=2005, project_id=8)]))
    for project_id, note_id in ((7, 701), (8, 801)):
        http_mocker.get(
            HttpRequest(f"{API_URL}/projects/{project_id}/merge_requests/5/notes", query_params=ANY_QUERY_PARAMS),
            _ok(
                [
                    {
                        "id": note_id,
                        "body": "ok",
                        "system": False,
                        "author": {"id": 11, "username": "alice"},
                        "created_at": "2026-06-20T10:00:00.000+00:00",
                        "updated_at": "2026-06-20T10:00:00.000+00:00",
                    }
                ]
            ),
        )

    output = read_stream(_CONNECTOR, "pull_request_notes", config)

    assert not output.errors
    by_project = {r.record.data["project_id"]: r.record.data for r in output.records}
    assert sorted(by_project) == [7, 8], f"should fan out both projects: {by_project!r}"
    assert (by_project[7]["id"], by_project[8]["id"]) == (701, 801)
    assert by_project[8]["mr_iid"] == 5 and by_project[8]["unique_key"] == "test-tenant:test-source:8:5:801"


@freezegun.freeze_time(_FROZEN)
@pytest.mark.parametrize("status", [402, 403, 404])
def test_a_project_scoped_error_on_one_merge_request_skips_it_not_the_stream(
    http_mocker: HttpMocker, status: int
) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([_mr(5), _mr(6)]))
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/merge_requests/5/notes", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body="", status_code=status),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/merge_requests/6/notes", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 601,
                    "body": "ok",
                    "system": False,
                    "author": {"id": 11, "username": "alice"},
                    "created_at": "2026-06-20T10:00:00.000+00:00",
                    "updated_at": "2026-06-20T10:00:00.000+00:00",
                }
            ]
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_notes", config)

    assert not output.errors
    assert [r.record.data["id"] for r in output.records] == [601]


def test_a_merge_request_listing_the_token_cannot_see_fails_as_a_config_error(http_mocker: HttpMocker) -> None:
    """The listing is per configured scope, like project discovery: a 404 there
    is a wrong path or a token without membership, not a project to skip."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), HttpResponse(body="", status_code=404))

    output = read_stream(_CONNECTOR, "pull_requests", config, expecting_exception=True)

    assert output.errors
    assert output.errors[-1].trace.error.failure_type == FailureType.config_error


@freezegun.freeze_time(_FROZEN)
def test_pull_requests_follow_the_link_header_to_the_next_page(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS),
        [
            HttpResponse(
                body=json.dumps([_mr(5)]),
                status_code=200,
                headers={"Link": f'<{_MRS_URL}?page=2&per_page=100>; rel="next"'},
            ),
            _ok([_mr(6)]),
        ],
    )

    output = read_stream(_CONNECTOR, "pull_requests", config)

    assert not output.errors
    assert [r.record.data["iid"] for r in output.records] == [5, 6]
    listing = _urls(http_mocker, "/merge_requests")
    assert len(listing) == 2 and "page=2" in listing[1], listing


@freezegun.freeze_time(_FROZEN)
def test_a_resumed_pull_requests_sync_asks_for_less_than_the_first(http_mocker: HttpMocker) -> None:
    """The start date is a floor paid once; a run carrying state asks only for
    what changed since, less the one-day lookback."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([_mr(5)]))

    first = read_stream(_CONNECTOR, "pull_requests", config)
    assert not first.errors
    assert first.state_messages, "an incremental read must emit state"
    resumed = read_stream(_CONNECTOR, "pull_requests", config, state=[m.state for m in first.state_messages][-1:])
    assert not resumed.errors, f"a resumed sync must not fail: {resumed.errors}"

    asked = _urls(http_mocker, "/merge_requests")
    assert "updated_after=2026-06-01" in asked[0], f"first run starts at the floor: {asked[0]}"
    assert "updated_after=2026-06-19" in asked[-1], f"a resumed run starts at stored state: {asked[-1]}"


@freezegun.freeze_time(_FROZEN)
def test_a_resumed_child_sync_resumes_the_parent_window_too(http_mocker: HttpMocker) -> None:
    """Every merge-request child rides the windowed parent with
    incremental_dependency, so the parent's cursor is persisted with the child
    and a later run lists only the merge requests updated since."""
    config = GitlabConfigBuilder().build()
    http_mocker.get(HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS), _ok([_mr(5)]))
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/merge_requests/5/notes", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 501,
                    "body": "ok",
                    "system": False,
                    "author": {"id": 11, "username": "alice"},
                    "created_at": "2026-06-20T10:00:00.000+00:00",
                    "updated_at": "2026-06-20T10:00:00.000+00:00",
                }
            ]
        ),
    )

    first = read_stream(_CONNECTOR, "pull_request_notes", config)
    assert not first.errors
    assert first.state_messages, "an incremental child must emit state"
    state = first.state_messages[-1].state.stream.stream_state.__dict__
    assert "parent_state" in state, f"the parent cursor must be persisted with the child: {state}"
    resumed = read_stream(_CONNECTOR, "pull_request_notes", config, state=[m.state for m in first.state_messages][-1:])
    assert not resumed.errors, f"a resumed sync must not fail: {resumed.errors}"

    listings = _urls(http_mocker, "/merge_requests?")
    assert "updated_after=2026-06-01" in listings[0], listings[0]
    assert "updated_after=2026-06-19" in listings[-1], (
        f"a resumed run lists from the parent's stored state: {listings[-1]}"
    )


# ── GraphQL ──────────────────────────────────────────────────────────────


def _graphql_body(stream_name: str, variables: dict[str, Any], cursor: str | None = None) -> dict[str, Any]:
    """The exact request body the manifest sends — POST mocks match on it."""
    stream = next(st for st in load_manifest(_CONNECTOR)["streams"] if st["name"] == stream_name)
    body = dict(stream["retriever"]["requester"]["request_body_json"])
    body["query"] = body["query"].rstrip("\n")
    body["variables"] = dict(variables)
    if cursor is not None:
        body["variables"]["cursor"] = cursor
    return body


_PROJECT_VARS = {"project": "acme/app", "updatedAfter": "2026-06-01T00:00:00Z"}


def _mock_roster(http_mocker: HttpMocker) -> None:
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _ok([_project()]))


@freezegun.freeze_time(_FROZEN)
def test_diff_stats_carry_real_integers_and_keep_pending_stats_null(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    _mock_roster(http_mocker)
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_graphql_body("pull_request_diff_stats", _PROJECT_VARS)),
        _ok(
            {
                "data": {
                    "project": {
                        "mergeRequests": {
                            "pageInfo": {"hasNextPage": True, "endCursor": "c1"},
                            "nodes": [
                                {
                                    "iid": "5",
                                    "updatedAt": "2026-06-20T10:00:00Z",
                                    "diffStatsSummary": {"additions": 120, "deletions": 7, "fileCount": 3},
                                }
                            ],
                        }
                    }
                }
            }
        ),
    )
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_graphql_body("pull_request_diff_stats", _PROJECT_VARS, cursor="c1")),
        _ok(
            {
                "data": {
                    "project": {
                        "mergeRequests": {
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                            "nodes": [{"iid": "6", "updatedAt": "2026-06-21T10:00:00Z", "diffStatsSummary": None}],
                        }
                    }
                }
            }
        ),
    )

    output = read_stream(_CONNECTOR, "pull_request_diff_stats", config)

    assert not output.errors
    by_iid = {r.record.data["mr_iid"]: r.record.data for r in output.records}
    assert (by_iid[5]["additions"], by_iid[5]["deletions"], by_iid[5]["files_changed"]) == (120, 7, 3)
    assert by_iid[5]["unique_key"] == "test-tenant:test-source:7:5"
    assert by_iid[6].get("additions") is None, "pending stats must not read as zero"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pull_request_diff_stats", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_pipelines_parse_the_global_id_and_link_the_merge_request(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    _mock_roster(http_mocker)
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_graphql_body("pipelines", _PROJECT_VARS)),
        _ok(
            {
                "data": {
                    "project": {
                        "pipelines": {
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                            "nodes": [
                                {
                                    "id": "gid://gitlab/Ci::Pipeline/9001",
                                    "iid": "12",
                                    "sha": "e" * 40,
                                    "ref": "feat",
                                    "status": "SUCCESS",
                                    "source": "merge_request_event",
                                    "createdAt": "2026-06-20T10:00:00Z",
                                    "updatedAt": "2026-06-20T10:05:00Z",
                                    "startedAt": "2026-06-20T10:00:10Z",
                                    "finishedAt": "2026-06-20T10:05:00Z",
                                    "duration": 290,
                                    "queuedDuration": 10.5,
                                    "retryable": False,
                                    "user": {"id": "gid://gitlab/User/11", "username": "alice", "name": "Alice"},
                                    "mergeRequest": {"iid": "5"},
                                },
                                {
                                    "id": "gid://gitlab/Ci::Pipeline/9002",
                                    "iid": "13",
                                    "sha": "f" * 40,
                                    "ref": "main",
                                    "status": "FAILED",
                                    "source": "push",
                                    "createdAt": "2026-06-21T10:00:00Z",
                                    "updatedAt": "2026-06-21T10:05:00Z",
                                    "startedAt": None,
                                    "finishedAt": None,
                                    "duration": None,
                                    "queuedDuration": None,
                                    "retryable": True,
                                    "user": None,
                                    "mergeRequest": None,
                                },
                            ],
                        }
                    }
                }
            }
        ),
    )

    output = read_stream(_CONNECTOR, "pipelines", config)

    assert not output.errors
    by_id = {r.record.data["id"]: r.record.data for r in output.records}
    first = by_id[9001]
    assert (first["iid"], first["status"], first["source"]) == (12, "success", "merge_request_event")
    assert (first["user_id"], first["user_username"], first["mr_iid"]) == (11, "alice", 5)
    assert first["project_id"] == 7 and first["repo_path"] == "acme/app"
    assert first["unique_key"] == "test-tenant:test-source:7:9001"
    second = by_id[9002]
    assert second["user_username"] == "" and second.get("user_id") is None and second.get("mr_iid") is None
    assert second["started_at"] == ""
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "pipelines", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_a_graphql_error_inside_a_200_fails_loudly(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    _mock_roster(http_mocker)
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_graphql_body("pipelines", _PROJECT_VARS)),
        _ok({"errors": [{"message": "Field 'queuedDuration' doesn't exist on type 'Pipeline'"}]}),
    )

    output = read_stream(_CONNECTOR, "pipelines", config, expecting_exception=True)

    assert output.errors, "a query-level GraphQL error must fail the stream"
    assert output.records == []
    assert any("queuedDuration" in (log.log.message or "") for log in output.logs), (
        "GitLab's own message must reach the log"
    )


@freezegun.freeze_time(_FROZEN)
def test_a_graphql_null_project_yields_no_rows_and_no_crash(http_mocker: HttpMocker) -> None:
    """The token can list a project it cannot query; GraphQL answers
    `project: null` with no errors array."""
    config = GitlabConfigBuilder().build()
    _mock_roster(http_mocker)
    http_mocker.post(
        HttpRequest(GRAPHQL_URL, body=_graphql_body("pipelines", _PROJECT_VARS)), _ok({"data": {"project": None}})
    )

    output = read_stream(_CONNECTOR, "pipelines", config)

    assert not output.errors
    assert output.records == []


# ── deployments, members, users ──────────────────────────────────────────


@freezegun.freeze_time(_FROZEN)
def test_deployments_keep_one_row_per_status(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().build()
    _mock_roster(http_mocker)
    deployment = {
        "id": 300,
        "iid": 4,
        "ref": "main",
        "sha": "f" * 40,
        "status": "running",
        "environment": {"id": 2, "name": "production"},
        "deployable": {"id": 77, "name": "deploy", "stage": "deploy", "pipeline": {"id": 9001}},
        "user": {"id": 11, "username": "alice", "name": "Alice"},
        "created_at": "2026-06-20T10:00:00.000+00:00",
        "updated_at": "2026-06-20T10:01:00.000+00:00",
    }
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/deployments", query_params=ANY_QUERY_PARAMS),
        _ok([deployment, {**deployment, "status": "success", "updated_at": "2026-06-20T10:05:00.000+00:00"}]),
    )

    output = read_stream(_CONNECTOR, "deployments", config)

    assert not output.errors
    keys = [r.record.data["unique_key"] for r in output.records]
    assert keys == ["test-tenant:test-source:7:300:running", "test-tenant:test-source:7:300:success"]
    rec = output.records[0].record.data
    assert (rec["environment_name"], rec["environment_id"], rec["pipeline_id"], rec["user_username"]) == (
        "production",
        2,
        9001,
        "alice",
    )
    listing = _urls(http_mocker, "/deployments")[0]
    assert "order_by=id" in listing and "updated_after=2026-06-01" in listing
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "deployments", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_environments_carry_the_tier_the_deployment_listing_omits(http_mocker: HttpMocker) -> None:
    """A deployment embeds its environment without a tier; the environment
    listing is where production is told apart from the rest."""
    config = GitlabConfigBuilder().build()
    _mock_roster(http_mocker)
    stamp = "2026-06-20T10:00:00.000+00:00"
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/environments", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 2,
                    "name": "production",
                    "slug": "production",
                    "state": "available",
                    "tier": "production",
                    "external_url": "https://app.example.com",
                    "created_at": stamp,
                    "updated_at": stamp,
                    "project": {"id": 7},
                },
                {
                    "id": 3,
                    "name": "review/feat",
                    "slug": "review-feat-abc",
                    "state": "stopped",
                    "tier": None,
                    "external_url": None,
                    "created_at": stamp,
                    "updated_at": stamp,
                    "project": {"id": 7},
                },
            ]
        ),
    )

    output = read_stream(_CONNECTOR, "environments", config)

    assert not output.errors
    by_id = {r.record.data["id"]: r.record.data for r in output.records}
    assert (by_id[2]["tier"], by_id[2]["state"], by_id[2]["project_id"]) == ("production", "available", 7)
    assert by_id[2]["repo_path"] == _project()["path_with_namespace"]
    assert by_id[3]["tier"] == "" and "project" not in by_id[3]
    assert by_id[2]["unique_key"] == "test-tenant:test-source:7:2"
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "environments", strict=True)


def test_a_402_on_one_project_deployments_skips_it_not_the_stream(http_mocker: HttpMocker) -> None:
    """Deployments are an edition feature: a project the licence does not
    cover answers 402 and is skipped, the rest of the roster is still read."""
    config = GitlabConfigBuilder().build()
    other = _project(
        id=8, path="api", path_with_namespace="acme/api", http_url_to_repo="https://gitlab.example.com/acme/api.git"
    )
    http_mocker.get(HttpRequest(_PROJECTS_URL, query_params=ANY_QUERY_PARAMS), _ok([_project(), other]))
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/7/deployments", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body="", status_code=402),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/8/deployments", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 301,
                    "iid": 1,
                    "ref": "main",
                    "sha": "a" * 40,
                    "status": "success",
                    "environment": {"id": 3, "name": "staging", "tier": "staging"},
                    "created_at": "2026-06-20T10:00:00.000+00:00",
                    "updated_at": "2026-06-20T10:01:00.000+00:00",
                }
            ]
        ),
    )

    output = read_stream(_CONNECTOR, "deployments", config)

    assert not output.errors
    assert [r.record.data["unique_key"] for r in output.records] == ["test-tenant:test-source:8:301:success"]


@freezegun.freeze_time(_FROZEN)
def test_members_are_read_per_configured_group_and_project(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().with_field("gitlab_projects", ["globex/site"]).build()
    member = {
        "id": 11,
        "username": "alice",
        "name": "Alice",
        "state": "active",
        "access_level": 30,
        "created_at": "2020-01-01T00:00:00.000+00:00",
        "expires_at": None,
        "avatar_url": "x",
        "web_url": "y",
    }
    http_mocker.get(HttpRequest(f"{API_URL}/groups/acme/members/all", query_params=ANY_QUERY_PARAMS), _ok([member]))
    http_mocker.get(
        HttpRequest(f"{API_URL}/projects/globex%2Fsite/members/all", query_params=ANY_QUERY_PARAMS), _ok([member])
    )

    output = read_stream(_CONNECTOR, "group_members", config)

    assert not output.errors
    keys = sorted(r.record.data["unique_key"] for r in output.records)
    assert keys == ["test-tenant:test-source:acme:11", "test-tenant:test-source:globex/site:11"]
    rec = output.records[0].record.data
    assert rec["email"] == "" and rec["public_email"] == "" and "avatar_url" not in rec
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "group_members", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_users_stay_silent_when_the_operator_opts_out(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().with_field("gitlab_instance_users", "false").build()

    output = read_stream(_CONNECTOR, "users", config)

    assert not output.errors
    assert output.records == []
    assert not _urls(http_mocker, "/users")


@freezegun.freeze_time(_FROZEN)
def test_a_directory_the_token_cannot_read_is_skipped_not_fatal(http_mocker: HttpMocker) -> None:
    """The directory is on by default and is enrichment, not a data path: a
    token without the right leaves it empty and the sync green."""
    http_mocker.get(
        HttpRequest(f"{API_URL}/users", query_params=ANY_QUERY_PARAMS), HttpResponse(body="", status_code=403)
    )

    output = read_stream(_CONNECTOR, "users", GitlabConfigBuilder().build())

    assert not output.errors
    assert output.records == []
    assert len(_urls(http_mocker, "/users")) == 1, "the default is to ask"


@freezegun.freeze_time(_FROZEN)
def test_users_walk_the_instance_with_keyset_pagination(http_mocker: HttpMocker) -> None:
    config = GitlabConfigBuilder().with_field("gitlab_instance_users", "true").build()
    http_mocker.get(
        HttpRequest(f"{API_URL}/users", query_params=ANY_QUERY_PARAMS),
        _ok(
            [
                {
                    "id": 11,
                    "username": "alice",
                    "name": "Alice",
                    "state": "active",
                    "email": "alice@example.com",
                    "public_email": None,
                    "commit_email": "alice@example.com",
                    "bot": False,
                    "is_admin": False,
                    "created_at": "2020-01-01T00:00:00.000+00:00",
                    "last_activity_on": "2026-06-30",
                    "bio": "x",
                }
            ]
        ),
    )

    output = read_stream(_CONNECTOR, "users", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["unique_key"] == "test-tenant:test-source:11"
    assert (rec["email"], rec["public_email"]) == ("alice@example.com", "")
    assert "bio" not in rec
    url = _urls(http_mocker, "/users")[0]
    assert "pagination=keyset" in url and "order_by=id" in url
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "users", strict=True)
