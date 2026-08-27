"""Mock-server tests for the `teams` stream.

Paginated GraphQL POST against the Teams namespace; the site scope is an ARI
derived from the configured cloud id, so no separate organization id is needed.

Coverage matrix rows: full_refresh_single_page, pagination_multi_page,
empty_page, tenant_source_stamping, schema_conformance, transformations,
error_retry (429), GraphQL-error-in-200.

Not applicable: incremental_state (full refresh — the Teams API exposes no
cursor anywhere), substream_partition (no parent), record_filter / error_ignore
(not declared).
"""

from __future__ import annotations

import json

from config import SITE_SCOPE, CompassConfigBuilder, child_query, request, team_ari
from connector_tests import HttpMocker, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "teams"
_CONNECTOR = "dev-portal/compass"
_QUERY = child_query(_STREAM)


def _req(**variables):
    return request(_QUERY, scopeId=SITE_SCOPE, **variables)


def _team(suffix: str, **overrides: object) -> dict:
    return load_fixture(__file__, "team.json", id=team_ari(suffix), **overrides)


def _page(teams: list[dict], *, cursor: str | None = None) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "team": {
                        "teamSearchV3": {
                            "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
                            "nodes": [{"team": t} for t in teams],
                        }
                    }
                }
            }
        ),
        status_code=200,
    )


def test_full_refresh_single_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_team("t1")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["team_id"] == team_ari("t1")
    assert rec["display_name"] == "Platform"
    assert rec["state"] == "ACTIVE"
    assert "team" not in rec
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_team_id_is_the_directory_join_key(http_mocker: HttpMocker):
    """`team_id` must stay the full ARI, unmodified.

    The UUID inside it is what Compass component ownership and Jira's
    atlassian-team field both reference, so this column is the join key for
    ownership across sources — rewriting or trimming it would break that join.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_team("t1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["team_id"].startswith("ari:cloud:identity::team/")


def test_member_count_absent_from_search_is_tolerated(http_mocker: HttpMocker):
    """The search field does not populate memberCount; that must not break the row."""
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_team("t1")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert output.records[0].record.data.get("member_count") in (None, 0, "")
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_pagination_multi_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_team("t1")], cursor="CURSOR_1"))
    http_mocker.post(_req(cursor="CURSOR_1"), _page([_team("t2")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert [r.record.data["team_id"] for r in output.records] == [team_ari("t1"), team_ari("t2")]


def test_empty_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([]))

    assert read_stream(_CONNECTOR, _STREAM, config).records == []


def test_tenant_source_stamping(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_team("t1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}:{config['insight_source_id']}:{team_ari('t1')}")


def test_graphql_error_in_http_200_fails_the_read(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _req(),
        HttpResponse(body=json.dumps({"errors": [{"message": "scopeId rejected"}], "data": None}), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors


def test_null_connection_fails_the_stream(http_mocker: HttpMocker):
    """`teamSearchV3` is not a union: an unusable scope returns a null connection.

    There is no error object to key on, so without the per-stream filter the
    directory would come back empty and the sync would look successful.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), HttpResponse(body=json.dumps({"data": {"team": {"teamSearchV3": None}}}), status_code=200))

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors


def test_error_retry_on_429(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _req(), [HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}), _page([_team("t1")])]
    )

    assert len(read_stream(_CONNECTOR, _STREAM, config).records) == 1
