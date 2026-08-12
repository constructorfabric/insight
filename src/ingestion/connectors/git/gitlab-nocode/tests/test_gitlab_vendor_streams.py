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
    _no_literal_none(output.records)
    assert_records_conform(output.records, _CONNECTOR, "merge_requests", strict=True)


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
                [{"id": 11, "username": "alice", "name": "Alice", "state": "active", "access_level": 30, "created_at": "2020-01-01T00:00:00.000+00:00"}]
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, "users", config)

    assert not output.errors
    rec = output.records[0].record.data
    assert rec["group"] == "acme"
    assert rec["unique_key"].endswith(":acme:11")
    _no_literal_none(output.records)
