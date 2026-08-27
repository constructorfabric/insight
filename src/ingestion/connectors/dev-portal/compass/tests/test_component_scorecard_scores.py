"""Mock-server tests for the `component_scorecard_scores` stream.

Substream over an inline ids-only scorecards parent, paginated on the child.
Records are extracted from `...appliedToComponents.edges`, where the score
hangs off the EDGE rather than the node.

Coverage matrix rows: full_refresh_single_page, pagination_multi_page,
empty_page, tenant_source_stamping, schema_conformance, transformations,
substream_partition, error_retry (429), GraphQL-error-in-200.

Not applicable: incremental_state (full refresh), record_filter / error_ignore
(not declared).
"""

from __future__ import annotations

import json

from config import CLOUD_ID, CompassConfigBuilder, child_query, component_ari, parent_query, request, scorecard_ari
from connector_tests import HttpMocker, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "component_scorecard_scores"
_CONNECTOR = "dev-portal/compass"


def _parent_req():
    return request(parent_query(_STREAM), cloudId=CLOUD_ID)


def _child_req(scorecard_suffix: str, **variables):
    return request(child_query(_STREAM), scorecardId=scorecard_ari(scorecard_suffix), **variables)


def _parent(*suffixes: str) -> HttpResponse:
    """The inline ids-only parent response: scorecard ids and nothing else."""
    return HttpResponse(
        body=json.dumps({"data": {"compass": {"scorecards": {"nodes": [{"id": scorecard_ari(s)} for s in suffixes]}}}}),
        status_code=200,
    )


def _edge(component_suffix: str, **overrides: object) -> dict:
    edge = load_fixture(__file__, "score_edge.json", **overrides)
    edge["node"] = {"id": component_ari(component_suffix)}
    return edge


def _child(scorecard_suffix: str, edges: list[dict], *, cursor: str | None = None) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "compass": {
                        "scorecard": {
                            "id": scorecard_ari(scorecard_suffix),
                            "appliedToComponents": {
                                "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
                                "edges": edges,
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
    http_mocker.post(_parent_req(), _parent("s1"))
    http_mocker.post(_child_req("s1"), _child("s1", [_edge("c1")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["scorecard_id"] == scorecard_ari("s1")
    assert rec["component_id"] == component_ari("c1")
    assert rec["total_score"] == 67
    assert rec["max_total_score"] == 100
    assert rec["status"] == "NEEDS_ATTENTION"
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_criteria_scores_carry_data_source_freshness(http_mocker: HttpMocker):
    """`dataSourceLastUpdated` must survive per criterion.

    A metric-backed criterion scores zero both when the component genuinely
    fails it and when the integration feeding the metric is absent. This
    timestamp is the only field that separates the two, so losing it would make
    every score unreadable downstream.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("s1"))
    http_mocker.post(_child_req("s1"), _child("s1", [_edge("c1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert [c["criterionId"] for c in rec["criteria_scores"]] == ["cr1", "cr2", "cr3"]
    assert rec["criteria_scores"][0]["dataSourceLastUpdated"] == "2026-08-10T14:51:00.233Z"
    assert rec["criteria_scores"][2]["status"] == "FAILING"
    # The raw union members are dropped once flattened.
    assert "node" not in rec
    assert "score" not in rec


def test_substream_partition_one_request_per_scorecard(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("s1", "s2"))
    http_mocker.post(_child_req("s1"), _child("s1", [_edge("c1")]))
    http_mocker.post(_child_req("s2"), _child("s2", [_edge("c2")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    pairs = {(r.record.data["scorecard_id"], r.record.data["component_id"]) for r in output.records}
    assert pairs == {(scorecard_ari("s1"), component_ari("c1")), (scorecard_ari("s2"), component_ari("c2"))}


def test_pagination_multi_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("s1"))
    http_mocker.post(_child_req("s1"), _child("s1", [_edge("c1")], cursor="CURSOR_1"))
    http_mocker.post(_child_req("s1", cursor="CURSOR_1"), _child("s1", [_edge("c2")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert {r.record.data["component_id"] for r in output.records} == {component_ari("c1"), component_ari("c2")}


def test_empty_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("s1"))
    http_mocker.post(_child_req("s1"), _child("s1", []))

    assert read_stream(_CONNECTOR, _STREAM, config).records == []


def test_tenant_source_stamping(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("s1"))
    http_mocker.post(_child_req("s1"), _child("s1", [_edge("c1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    # Keyed on the pair, so one component scored by several scorecards does not collide.
    assert rec["unique_key"] == (
        f"{config['insight_tenant_id']}:{config['insight_source_id']}:{scorecard_ari('s1')}:{component_ari('c1')}"
    )


def test_graphql_error_in_http_200_fails_the_read(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _parent_req(),
        HttpResponse(body=json.dumps({"errors": [{"message": "OptInException"}], "data": None}), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors


def test_union_error_on_applied_components_fails(http_mocker: HttpMocker):
    """The beta opt-in being withdrawn surfaces here, not as a top-level error.

    `appliedToComponents` is a union, so a refusal is a successful body with a
    `message` and no `edges`. Left unhandled the stream would report zero
    scores for every scorecard and look healthy.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("s1"))
    http_mocker.post(
        _child_req("s1"),
        HttpResponse(
            body=json.dumps(
                {
                    "data": {
                        "compass": {
                            "scorecard": {
                                "id": scorecard_ari("s1"),
                                "appliedToComponents": {"message": "EXPERIMENTAL field requires opt-in"},
                            }
                        }
                    }
                }
            ),
            status_code=200,
        ),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors


def test_error_retry_on_429(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _parent_req(), [HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}), _parent("s1")]
    )
    http_mocker.post(_child_req("s1"), _child("s1", [_edge("c1")]))

    assert len(read_stream(_CONNECTOR, _STREAM, config).records) == 1
