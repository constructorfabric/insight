"""Mock-server tests for the `team_members` stream.

Substream over an inline ids-only teams parent, paginated per team. Fetching
members per team rather than through the bulk resolve is deliberate: in the bulk
form the nested member list is truncated and `hasNextPage` is true for every
team, including teams whose members all fit.

Coverage matrix rows: full_refresh_single_page, pagination_multi_page,
empty_page, tenant_source_stamping, schema_conformance, transformations,
substream_partition, error_retry (429), GraphQL-error-in-200.

Not applicable: incremental_state (full refresh), record_filter / error_ignore
(not declared).
"""

from __future__ import annotations

import json

from config import CLOUD_ID, SITE_SCOPE, CompassConfigBuilder, child_query, parent_query, request, team_ari, user_ari
from connector_tests import HttpMocker, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "team_members"
_CONNECTOR = "dev-portal/compass"


def _parent_req():
    return request(parent_query(_STREAM), scopeId=SITE_SCOPE)


def _child_req(team_suffix: str, **variables):
    return request(child_query(_STREAM), teamId=team_ari(team_suffix), siteId=CLOUD_ID, **variables)


def _parent(*suffixes: str) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "team": {
                        "teamSearchV3": {
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                            "nodes": [{"team": {"id": team_ari(s)}} for s in suffixes],
                        }
                    }
                }
            }
        ),
        status_code=200,
    )


def _member(user_suffix: str, **overrides: object) -> dict:
    member = load_fixture(__file__, "team_member.json", **overrides)
    member["member"] = dict(member["member"], id=user_ari(user_suffix))
    return member


def _child(team_suffix: str, members: list[dict], *, cursor: str | None = None) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "team": {
                        "teamV2": {
                            "id": team_ari(team_suffix),
                            "members": {
                                "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
                                "nodes": members,
                            },
                        }
                    }
                }
            }
        ),
        status_code=200,
    )


def test_full_refresh_single_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("t1"))
    http_mocker.post(_child_req("t1"), _child("t1", [_member("u1")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["team_id"] == team_ari("t1")
    assert rec["member_id"] == user_ari("u1")
    assert rec["role"] == "REGULAR"
    assert rec["membership_state"] == "FULL_MEMBER"
    assert "member" not in rec
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_member_id_is_the_account_join_key(http_mocker: HttpMocker):
    """People must be joinable by account id — the API exposes no email here."""
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("t1"))
    http_mocker.post(_child_req("t1"), _child("t1", [_member("u1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["member_id"].startswith("ari:cloud:identity::user/")
    assert "email" not in rec


def test_inactive_accounts_are_kept(http_mocker: HttpMocker):
    """Deactivated accounts stay listed as members; the status must be preserved."""
    config = CompassConfigBuilder().build()
    inactive = _member("u2")
    inactive["member"]["accountStatus"] = "inactive"
    http_mocker.post(_parent_req(), _parent("t1"))
    http_mocker.post(_child_req("t1"), _child("t1", [inactive]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["account_status"] == "inactive"


def test_substream_partition_one_request_per_team(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("t1", "t2"))
    http_mocker.post(_child_req("t1"), _child("t1", [_member("u1")]))
    http_mocker.post(_child_req("t2"), _child("t2", [_member("u1")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    # The same person in two teams yields two rows with distinct keys —
    # membership is many-to-many, so a person-keyed row would lose one.
    keys = {r.record.data["unique_key"] for r in output.records}
    assert len(keys) == 2
    assert {r.record.data["team_id"] for r in output.records} == {team_ari("t1"), team_ari("t2")}


def test_pagination_multi_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("t1"))
    http_mocker.post(_child_req("t1"), _child("t1", [_member("u1")], cursor="CURSOR_1"))
    http_mocker.post(_child_req("t1", cursor="CURSOR_1"), _child("t1", [_member("u2")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert {r.record.data["member_id"] for r in output.records} == {user_ari("u1"), user_ari("u2")}


def test_empty_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("t1"))
    http_mocker.post(_child_req("t1"), _child("t1", []))

    assert read_stream(_CONNECTOR, _STREAM, config).records == []


def test_tenant_source_stamping(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("t1"))
    http_mocker.post(_child_req("t1"), _child("t1", [_member("u1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (
        f"{config['insight_tenant_id']}:{config['insight_source_id']}:{team_ari('t1')}:{user_ari('u1')}"
    )


def test_graphql_error_in_http_200_fails_the_read(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _parent_req(),
        HttpResponse(body=json.dumps({"errors": [{"message": "team not visible"}], "data": None}), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors


def test_error_retry_on_429(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _parent_req(), [HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}), _parent("t1")]
    )
    http_mocker.post(_child_req("t1"), _child("t1", [_member("u1")]))

    assert len(read_stream(_CONNECTOR, _STREAM, config).records) == 1
