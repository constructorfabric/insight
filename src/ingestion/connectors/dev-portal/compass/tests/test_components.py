"""Mock-server tests for the `components` stream.

Paginated GraphQL POST: cursor injected into `variables.cursor`, records
extracted from `data.compass.searchComponents.nodes`, the nested `component`
object hoisted into flat columns with links / labels / event sources /
relationships carried as JSON.

Coverage matrix rows: full_refresh_single_page, pagination_multi_page,
empty_page, tenant_source_stamping, schema_conformance, transformations,
error_retry (429), plus the two GraphQL-specific hazards: an error body served
with HTTP 200, and a `QueryError` union member where a connection was expected.

Not applicable: incremental_state (stream is full refresh),
substream_partition (no parent), record_filter (none declared),
error_ignore (the manifest ignores no status codes — a GraphQL error must fail
loudly, not be swallowed).
"""

from __future__ import annotations

import json

from config import CLOUD_ID, CompassConfigBuilder, child_query, component_ari, request
from connector_tests import HttpMocker, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "components"
_CONNECTOR = "dev-portal/compass"
_QUERY = child_query(_STREAM)


def _req(**variables):
    """Matcher for one catalog page — pass `cursor` to match the follow-up page."""
    return request(_QUERY, cloudId=CLOUD_ID, **variables)


def _component(suffix: str, **overrides: object) -> dict:
    return load_fixture(__file__, "component.json", id=component_ari(suffix), **overrides)


def _page(components: list[dict], *, cursor: str | None = None) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "compass": {
                        "searchComponents": {
                            "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
                            "nodes": [{"component": c} for c in components],
                        }
                    }
                }
            }
        ),
        status_code=200,
    )


def test_full_refresh_single_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_component("c1")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["component_id"] == component_ari("c1")
    assert rec["name"] == "billing-api"
    assert rec["component_type"] == "SERVICE"
    assert rec["owner_team_id"] == "ari:cloud:identity::team/t1"
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_transformations_flatten_nested_collections(http_mocker: HttpMocker):
    """links / labels / event_sources / relationships survive as structured JSON."""
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_component("c1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["labels"] == ["tier-1"]
    assert [link["type"] for link in rec["links"]] == ["REPOSITORY"]
    assert rec["links"][0]["url"] == "https://github.com/example-org/billing-api"
    assert [src["eventType"] for src in rec["event_sources"]] == ["DEPLOYMENT"]
    assert [rel["relationshipType"] for rel in rec["relationships"]] == ["DEPENDS_ON"]
    # The nested relationship list cannot be paginated, so the truncation flag is
    # the only signal that edges were dropped. It must never be silently absent.
    assert rec["relationships_truncated"] is False
    # The raw nested object is removed rather than duplicated alongside the flat columns.
    assert "component" not in rec


def test_truncated_relationships_stay_visible(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    component = _component("c1")
    component["relationships"]["pageInfo"]["hasNextPage"] = True
    http_mocker.post(_req(), _page([component]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["relationships_truncated"] is True


def test_pagination_multi_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    # Every page hits the same URL; the cursor travels in the POST body. Two
    # separate body matchers are therefore the assertion that the paginator
    # injected the cursor into `variables` and not somewhere else.
    http_mocker.post(_req(), _page([_component("c1")], cursor="CURSOR_1"))
    http_mocker.post(_req(cursor="CURSOR_1"), _page([_component("c2")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    ids = [r.record.data["component_id"] for r in output.records]
    assert ids == [component_ari("c1"), component_ari("c2")]


def test_empty_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([]))

    assert read_stream(_CONNECTOR, _STREAM, config).records == []


def test_tenant_source_stamping(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _page([_component("c1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["data_source"] == "insight_compass"
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}:{config['insight_source_id']}:{component_ari('c1')}")


def test_graphql_error_in_http_200_fails_the_read(http_mocker: HttpMocker):
    """The whole point of the body-predicate response filter.

    Atlassian answers a rejected query with HTTP 200 and an `errors[]` array. A
    status-code-only error handler sees success, the extractor finds no `nodes`,
    and the sync reports zero records while the real cause never surfaces.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _req(),
        HttpResponse(
            body=json.dumps({"errors": [{"message": "Field 'x' is marked as EXPERIMENTAL"}], "data": None}),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors, "a GraphQL error served as HTTP 200 must fail the stream"


def test_query_error_union_member_fails_the_catalog(http_mocker: HttpMocker):
    """`searchComponents` returns a union, and its error member has no `nodes`.

    This body carries no top-level `errors`, so the catch-all filter cannot see
    it; without the per-stream filter the extractor would find nothing and the
    whole catalog would report zero components as a successful sync. For the
    catalog that is never an acceptable outcome, so it must fail loudly.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _req(),
        HttpResponse(
            body=json.dumps({"data": {"compass": {"searchComponents": {"message": "Not found"}}}}), status_code=200
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors, "a union QueryError on the catalog must fail the stream"


def test_error_retry_on_429(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _req(), [HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}), _page([_component("c1")])]
    )

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
