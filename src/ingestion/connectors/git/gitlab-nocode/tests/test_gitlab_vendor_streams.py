"""Mock-server tests for the GitLab vendor streams added alongside the proxy.

Covers the merge_requests window emission, the approvals 402 tolerance
(premium feature ≠ error), and — everywhere — the rule from the
github-directory incident: an absent nested field must surface as '' or a
real null, never as the literal text "None".

Coverage matrix rows: full_refresh_single_page, incremental_state,
tenant_source_stamping, schema_conformance, error_ignore (402/403/404),
transformations (None-guard).
"""

from __future__ import annotations

import json

import freezegun
from config import GITLAB_URL, GitlabNocodeConfigBuilder

from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    read_stream,
)
from connector_tests.source import load_manifest

_CONNECTOR = "git/gitlab-nocode"
_MRS_URL = f"{GITLAB_URL}/api/v4/groups/acme/merge_requests"
_FROZEN = "2026-07-01T00:00:00Z"


def _no_literal_none(records) -> None:
    """The github-directory bug class: a Jinja `none` rendered into a
    value_type: string field stores the four-character text \"None\"."""
    for r in records:
        for key, value in r.record.data.items():
            assert value != "None", f"literal 'None' leaked into {key}"


def _mr(mr_id: int, *, author: dict | None, merged_by: dict | None) -> dict:
    return {
        "id": mr_id,
        "iid": mr_id,
        "project_id": 7,
        "state": "merged",
        "draft": False,
        "title": f"MR {mr_id}",
        "description": "d" * 3000,
        "author": author,
        "merged_by": merged_by,
        "source_branch": "feat",
        "target_branch": "main",
        "sha": "e" * 40,
        "created_at": "2026-06-10T10:00:00.000+00:00",
        "updated_at": "2026-06-20T10:00:00.000+00:00",
        "merged_at": "2026-06-20T10:00:00.000+00:00",
        "user_notes_count": 2,
    }


@freezegun.freeze_time(_FROZEN)
def test_merge_requests_and_the_none_guard(http_mocker: HttpMocker) -> None:
    """A deleted account leaves author/merged_by null; the hoisted usernames
    must come out '' — never the text \"None\"."""
    config = GitlabNocodeConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    _mr(1, author={"username": "alice"}, merged_by=None),
                    _mr(2, author=None, merged_by=None),
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "merge_requests", config)

    assert not output.errors
    assert len(output.records) == 2
    by_id = {r.record.data["id"]: r.record.data for r in output.records}
    assert by_id[1]["author_username"] == "alice"
    assert by_id[1]["merged_by_username"] == ""
    assert by_id[2]["author_username"] == ""
    assert len(by_id[1]["description"]) == 2048
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "merge_requests", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_merge_request_commits_store_only_the_edge(http_mocker: HttpMocker) -> None:
    """MR<->commit membership is vendor-only; the commit payload belongs to
    the proxy streams, so bronze keeps the sha and the MR identity alone."""
    config = GitlabNocodeConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([_mr(5, author={"username": "alice"}, merged_by=None)]),
            status_code=200,
        ),
    )
    http_mocker.get(
        HttpRequest(
            f"{GITLAB_URL}/api/v4/projects/7/merge_requests/5/commits",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
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
                        "extended_trailers": {},
                        "web_url": "https://gitlab.example.com/acme/app/-/commit/" + "a" * 40,
                    }
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "merge_request_commits", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["sha"] == "a" * 40
    assert rec["mr_iid"] == 5
    assert rec["project_id"] == 7
    assert rec["unique_key"].endswith(f":7:5:{'a' * 40}")
    assert "message" not in rec and "author_name" not in rec
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "merge_request_commits", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_approvals_402_is_a_missing_feature_not_an_error(http_mocker: HttpMocker) -> None:
    config = GitlabNocodeConfigBuilder().build()
    # Windowed MR parent listing.
    http_mocker.get(
        HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([_mr(5, author={"username": "alice"}, merged_by=None)]),
            status_code=200,
        ),
    )
    http_mocker.get(
        HttpRequest(
            f"{GITLAB_URL}/api/v4/projects/7/merge_requests/5/approvals",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(body="", status_code=402),
    )

    output = read_stream(_CONNECTOR, "merge_request_approvals", config)

    assert not output.errors
    assert len(output.records) == 0


@freezegun.freeze_time(_FROZEN)
def test_users_full_refresh(http_mocker: HttpMocker) -> None:
    config = GitlabNocodeConfigBuilder().build()
    http_mocker.get(
        HttpRequest(f"{GITLAB_URL}/api/v4/groups/acme/members/all", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [{"id": 11, "username": "alice", "name": "Alice", "state": "active", "access_level": 30, "created_at": "2020-01-01T00:00:00.000+00:00", "public_email": "alice@example.com"}]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "users", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["group"] == "acme"
    assert rec["unique_key"].endswith(":acme:11")
    assert rec["public_email"] == "alice@example.com"
    _no_literal_none(output.records)


@freezegun.freeze_time(_FROZEN)
def test_notes_store_trimmed_bodies_and_inline_position(http_mocker: HttpMocker) -> None:
    """The silver comments model consumes body/author_id/position; bodies are
    trimmed to 2048 chars, and absent nested objects surface as ''/null."""
    config = GitlabNocodeConfigBuilder().build()
    http_mocker.get(
        HttpRequest(_MRS_URL, query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps([_mr(5, author={"username": "alice"}, merged_by=None)]),
            status_code=200,
        ),
    )
    http_mocker.get(
        HttpRequest(
            f"{GITLAB_URL}/api/v4/projects/7/merge_requests/5/notes",
            query_params=ANY_QUERY_PARAMS,
        ),
        HttpResponse(
            body=json.dumps(
                [
                    {
                        "id": 501,
                        "body": "n" * 3000,
                        "system": False,
                        "resolvable": True,
                        "resolved": False,
                        "author": {"id": 11, "username": "alice"},
                        "position": {"new_path": "src/a.py", "new_line": 3},
                        "created_at": "2026-06-20T10:00:00.000+00:00",
                        "updated_at": "2026-06-20T10:00:00.000+00:00",
                    },
                    {
                        "id": 502,
                        "body": "short",
                        "system": True,
                        "author": None,
                        "created_at": "2026-06-20T10:00:00.000+00:00",
                        "updated_at": "2026-06-20T10:00:00.000+00:00",
                    },
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "merge_request_notes", config)

    assert not output.errors
    by_id = {r.record.data["id"]: r.record.data for r in output.records}
    assert len(by_id[501]["body"]) == 2048
    assert by_id[501]["author_id"] == 11
    assert by_id[501]["position_new_path"] == "src/a.py"
    assert by_id[501]["position_new_line"] == 3
    assert by_id[502]["body"] == "short"
    assert by_id[502]["position_new_path"] == ""
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "merge_request_notes", strict=True)


def _diff_stats_body(cursor: str | None = None) -> dict:
    """The exact request body the manifest sends — POST mocks match on it."""
    manifest = load_manifest(_CONNECTOR)
    stream = next(st for st in manifest["streams"] if st["name"] == "merge_request_diff_stats")
    body = dict(stream["retriever"]["requester"]["request_body_json"])
    # The CDK's interpolation strips the YAML block scalar's trailing newline
    # at send time; the raw manifest keeps it.
    body["query"] = body["query"].rstrip("\n")
    body["variables"] = {"project": "acme/app", "updatedAfter": "2026-06-01T00:00:00Z"}
    if cursor is not None:
        body["variables"]["cursor"] = cursor
    return body


@freezegun.freeze_time(_FROZEN)
def test_merge_request_diff_stats_carry_real_integers(http_mocker: HttpMocker) -> None:
    """REST exposes only a capped `changes_count` STRING; GraphQL carries real
    integers, and a null diffStatsSummary (stats computed asynchronously) must
    stay null rather than becoming a zero the metrics would trust."""
    config = GitlabNocodeConfigBuilder().build()
    http_mocker.get(
        HttpRequest(f"{GITLAB_URL}/api/v4/groups/acme/projects", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [{"id": 7, "path_with_namespace": "acme/app", "last_activity_at": "2026-06-20T10:00:00.000+00:00"}]
            ),
            status_code=200,
        ),
    )
    http_mocker.post(
        HttpRequest(f"{GITLAB_URL}/api/graphql", body=_diff_stats_body()),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "project": {
                            "mergeRequests": {
                                "pageInfo": {"hasNextPage": False, "endCursor": None},
                                "nodes": [
                                    {
                                        "iid": "5",
                                        "updatedAt": "2026-06-20T10:00:00Z",
                                        "diffStatsSummary": {"additions": 120, "deletions": 7, "fileCount": 3},
                                    },
                                    {
                                        "iid": "6",
                                        "updatedAt": "2026-06-21T10:00:00Z",
                                        "diffStatsSummary": None,
                                    },
                                ],
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "merge_request_diff_stats", config)

    assert not output.errors
    by_iid = {r.record.data["mr_iid"]: r.record.data for r in output.records}
    assert (by_iid[5]["additions"], by_iid[5]["deletions"], by_iid[5]["files_changed"]) == (120, 7, 3)
    assert by_iid[5]["project_id"] == 7
    assert by_iid[5]["unique_key"].endswith(":7:5")
    # An absent field lands as SQL NULL; a zero would read as "MR with no
    # changes", which is a different fact from "stats not computed yet".
    assert "additions" not in by_iid[6], "pending stats must not read as zero"
    assert "diffStatsSummary" not in by_iid[5]
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "merge_request_diff_stats", strict=True)


@freezegun.freeze_time(_FROZEN)
def test_empty_groups_syncs_the_full_instance(http_mocker: HttpMocker) -> None:
    """gitlab_groups empty flips discovery to keyset-paginated /projects —
    every project the token can see, no offset ceiling."""
    config = GitlabNocodeConfigBuilder().build()
    config["gitlab_groups"] = []
    http_mocker.get(
        HttpRequest(f"{GITLAB_URL}/api/v4/projects", query_params=ANY_QUERY_PARAMS),
        HttpResponse(
            body=json.dumps(
                [
                    {
                        "id": 7,
                        "path_with_namespace": "acme/app",
                        "name": "app",
                        "default_branch": "main",
                        "archived": False,
                        "visibility": "private",
                        "http_url_to_repo": "https://gitlab.example.com/acme/app.git",
                        "web_url": "https://gitlab.example.com/acme/app",
                        "created_at": "2020-01-01T00:00:00.000+00:00",
                        "last_activity_at": "2026-06-20T10:00:00.000+00:00",
                    }
                ]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "repositories", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["project_id"] == 7
    _no_literal_none(output.records)
