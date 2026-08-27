"""Mock-server tests for the `scorecards` stream.

Unpaginated GraphQL POST (the field takes no cursor), records extracted from
`data.compass.scorecards.nodes`, `changeMetadata` hoisted into flat timestamp
columns and `criterias` kept as JSON.

Coverage matrix rows: full_refresh_single_page, empty_page,
tenant_source_stamping, schema_conformance, transformations, error_retry (429),
GraphQL-error-in-200.

Not applicable: pagination_multi_page (the field exposes no cursor — see the
NoPagination paginator), incremental_state (full refresh), substream_partition
(no parent), record_filter / error_ignore (not declared).
"""

from __future__ import annotations

import json

from config import CLOUD_ID, CompassConfigBuilder, child_query, request, scorecard_ari
from connector_tests import HttpMocker, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "scorecards"
_CONNECTOR = "dev-portal/compass"


def _req():
    return request(child_query(_STREAM), cloudId=CLOUD_ID)


def _scorecard(suffix: str, **overrides: object) -> dict:
    return load_fixture(__file__, "scorecard.json", id=scorecard_ari(suffix), **overrides)


def _response(scorecards: list[dict]) -> HttpResponse:
    return HttpResponse(body=json.dumps({"data": {"compass": {"scorecards": {"nodes": scorecards}}}}), status_code=200)


def test_full_refresh_single_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _response([_scorecard("s1")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["scorecard_id"] == scorecard_ari("s1")
    assert rec["state"] == "PUBLISHED"
    assert rec["importance"] == "REQUIRED"
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_criteria_and_change_metadata(http_mocker: HttpMocker):
    """Criteria stay JSON; the change timestamps become columns.

    `criterias` is deliberately not exploded into typed columns: the criterion
    subtypes are an open set. `last_user_modification_at` is the only signal the
    API gives that a definition changed at all — there is no diff and no version.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _response([_scorecard("s1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["created_at"] == "2025-11-10T16:39:15.852Z"
    assert rec["last_user_modification_at"] == "2026-01-04T09:12:00.000Z"
    assert "changeMetadata" not in rec
    weights = [c["weight"] for c in rec["criterias"]]
    assert sum(weights) == 100, "criterion weights are normalised to 100 — the reason a bare score is ambiguous"
    assert {c["__typename"] for c in rec["criterias"]} == {
        "CompassHasOwnerScorecardCriteria",
        "CompassHasLinkScorecardCriteria",
        "CompassHasDescriptionScorecardCriteria",
    }


def test_empty_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _response([]))

    assert read_stream(_CONNECTOR, _STREAM, config).records == []


def test_tenant_source_stamping(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_req(), _response([_scorecard("s1")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}:{config['insight_source_id']}:{scorecard_ari('s1')}")


def test_graphql_error_in_http_200_fails_the_read(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _req(),
        HttpResponse(body=json.dumps({"errors": [{"message": "OptInException"}], "data": None}), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors


def test_error_retry_on_429(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _req(), [HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}), _response([_scorecard("s1")])]
    )

    assert len(read_stream(_CONNECTOR, _STREAM, config).records) == 1
