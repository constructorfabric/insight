"""Mock-server tests for the `deployment_events` stream.

Substream over an inline ids-only components parent, incremental on
`last_updated`. The cursor is client-side: the source exposes no usable
server-side date filter, so pages come back newest-first and the CDK filters
them against state.

Coverage matrix rows: full_refresh_single_page, pagination_multi_page,
empty_page, tenant_source_stamping, schema_conformance, transformations,
substream_partition, incremental_state, error_retry (429),
GraphQL-error-in-200.

Not applicable: record_filter (none declared beyond the cursor filter),
error_ignore (the manifest ignores no status codes).
"""

from __future__ import annotations

import json

from config import CLOUD_ID, CompassConfigBuilder, child_query, component_ari, parent_query, request
from connector_tests import HttpMocker, HttpResponse, assert_records_conform, load_fixture, read_stream
from freezegun import freeze_time

_STREAM = "deployment_events"
_CONNECTOR = "dev-portal/compass"
_NOW = "2026-06-20T00:00:00Z"


def _parent_req():
    return request(parent_query(_STREAM), cloudId=CLOUD_ID)


def _child_req(component_suffix: str, **variables):
    return request(child_query(_STREAM), componentId=component_ari(component_suffix), **variables)


def _parent(*suffixes: str) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "compass": {
                        "searchComponents": {
                            "pageInfo": {"hasNextPage": False, "endCursor": None},
                            "nodes": [{"component": {"id": component_ari(s)}} for s in suffixes],
                        }
                    }
                }
            }
        ),
        status_code=200,
    )


def _event(usn: int, last_updated: str, **overrides: object) -> dict:
    return load_fixture(
        __file__, "deployment_event.json", updateSequenceNumber=usn, lastUpdated=last_updated, **overrides
    )


def _child(component_suffix: str, events: list[dict], *, cursor: str | None = None) -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            {
                "data": {
                    "compass": {
                        "component": {
                            "id": component_ari(component_suffix),
                            "events": {
                                "pageInfo": {"hasNextPage": cursor is not None, "endCursor": cursor},
                                "nodes": events,
                            },
                        }
                    }
                }
            }
        ),
        status_code=200,
    )


@freeze_time(_NOW)
def test_full_refresh_single_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [_event(1750000000123, "2026-06-15T10:20:30.123Z")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 1
    rec = output.records[0].record.data
    assert rec["component_id"] == component_ari("c1")
    assert rec["event_type"] == "DEPLOYMENT"
    assert rec["last_updated"] == "2026-06-15T10:20:30.123Z"
    assert rec["update_sequence_number"] == 1750000000123
    assert_records_conform(output.records, _CONNECTOR, _STREAM)


@freeze_time(_NOW)
def test_transformations_flatten_deployment_properties(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [_event(1750000000123, "2026-06-15T10:20:30.123Z")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["state"] == "SUCCESSFUL"
    # Release metrics must key on this column — a DEPLOYMENT event with a
    # non-production category is build or pre-production activity, not a release.
    assert rec["environment_category"] == "PRODUCTION"
    assert rec["environment_id"] == "prod"
    assert rec["started_at"] == "2026-06-15T10:15:00.000Z"
    assert rec["completed_at"] == "2026-06-15T10:20:30.123Z"
    assert rec["sequence_number"] == 4242
    assert rec["pipeline"]["pipelineId"] == "4242"
    assert "deploymentProperties" not in rec


@freeze_time(_NOW)
def test_in_progress_event_keeps_null_completion(http_mocker: HttpMocker):
    """A terminal state may never arrive; the row must still be emitted."""
    config = CompassConfigBuilder().build()
    event = _event(1750000000999, "2026-06-16T08:00:00.000Z")
    event["deploymentProperties"]["state"] = "IN_PROGRESS"
    event["deploymentProperties"]["completedAt"] = None
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [event]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["state"] == "IN_PROGRESS"
    assert rec.get("completed_at") is None


@freeze_time(_NOW)
def test_unique_key_excludes_the_cursor(http_mocker: HttpMocker):
    """One event rewritten in place must replace its row, not fork a new one.

    Compass updates an event as the deployment progresses, so the key is
    (component, updateSequenceNumber). Including `last_updated` would make every
    progress update a separate bronze row.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [_event(1750000000123, "2026-06-15T10:20:30.123Z")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["unique_key"] == (
        f"{config['insight_tenant_id']}:{config['insight_source_id']}:{component_ari('c1')}:1750000000123"
    )
    assert "2026-06-15" not in rec["unique_key"]


@freeze_time(_NOW)
def test_substream_partition_one_request_per_component(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1", "c2"))
    http_mocker.post(_child_req("c1"), _child("c1", [_event(1, "2026-06-15T10:00:00.000Z")]))
    http_mocker.post(_child_req("c2"), _child("c2", [_event(2, "2026-06-16T10:00:00.000Z")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert {r.record.data["component_id"] for r in output.records} == {component_ari("c1"), component_ari("c2")}


@freeze_time(_NOW)
def test_pagination_multi_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [_event(2, "2026-06-16T10:00:00.000Z")], cursor="CURSOR_1"))
    http_mocker.post(_child_req("c1", cursor="CURSOR_1"), _child("c1", [_event(1, "2026-06-15T10:00:00.000Z")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert {r.record.data["update_sequence_number"] for r in output.records} == {1, 2}


@freeze_time(_NOW)
def test_incremental_state_emitted_and_resume_filters(http_mocker: HttpMocker):
    """State advances, and a resumed read drops events at or below the cursor.

    The filtering is client-side by construction: the source's own
    `timeParameters` window is unusable, so the manifest sets
    `is_client_side_incremental` and the CDK does the comparison. This test is
    what proves the cursor is actually observed — without the AddFields hoist of
    `lastUpdated` to a top-level column the cursor sees nothing and every sync
    re-reads the whole window.
    """
    config = CompassConfigBuilder().build()
    old = _event(1, "2026-06-10T00:00:00.000Z")
    new = _event(2, "2026-06-18T00:00:00.000Z")

    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [new, old]))
    first = read_stream(_CONNECTOR, _STREAM, config)

    assert {r.record.data["update_sequence_number"] for r in first.records} == {1, 2}
    assert first.state_messages, "an incremental stream must emit state"

    http_mocker.clear_all_matchers()
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [new, old]))
    state = [m.state for m in first.state_messages][-1:]
    resumed = read_stream(_CONNECTOR, _STREAM, config, state=state)

    resumed_usns = {r.record.data["update_sequence_number"] for r in resumed.records}
    assert 1 not in resumed_usns, "an event older than the cursor must be filtered out"
    assert len(resumed.records) < len(first.records)


@freeze_time(_NOW)
def test_empty_page(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", []))

    assert read_stream(_CONNECTOR, _STREAM, config).records == []


@freeze_time(_NOW)
def test_tenant_source_stamping(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1"))
    http_mocker.post(_child_req("c1"), _child("c1", [_event(1, "2026-06-15T10:00:00.000Z")]))

    rec = read_stream(_CONNECTOR, _STREAM, config).records[0].record.data

    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    assert rec["data_source"] == "insight_compass"


@freeze_time(_NOW)
def test_graphql_error_in_http_200_fails_the_read(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _parent_req(),
        HttpResponse(body=json.dumps({"errors": [{"message": "component not found"}], "data": None}), status_code=200),
    )

    output = read_stream(_CONNECTOR, _STREAM, config, expecting_exception=True)

    assert output.records == []
    assert output.errors


@freeze_time(_NOW)
def test_unreadable_component_is_skipped_not_fatal(http_mocker: HttpMocker):
    """A component deleted between the parent sweep and this read is ordinary.

    It comes back as a `QueryError` union member with no `events` and no
    `pageInfo` — so the paginator paths have to be null-safe, and the stream
    handler IGNOREs it rather than failing. Unlike the catalog query, one
    unreadable component out of thousands must not take the sync down: the
    remaining partitions still have to produce their events.
    """
    config = CompassConfigBuilder().build()
    http_mocker.post(_parent_req(), _parent("c1", "c2"))
    http_mocker.post(
        _child_req("c1"),
        HttpResponse(
            body=json.dumps({"data": {"compass": {"component": {"message": "Component not found"}}}}), status_code=200
        ),
    )
    http_mocker.post(_child_req("c2"), _child("c2", [_event(7, "2026-06-15T10:00:00.000Z")]))

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert not output.errors
    assert [r.record.data["component_id"] for r in output.records] == [component_ari("c2")]


@freeze_time(_NOW)
def test_error_retry_on_429(http_mocker: HttpMocker):
    config = CompassConfigBuilder().build()
    http_mocker.post(
        _parent_req(), [HttpResponse(body="{}", status_code=429, headers={"Retry-After": "0"}), _parent("c1")]
    )
    http_mocker.post(_child_req("c1"), _child("c1", [_event(1, "2026-06-15T10:00:00.000Z")]))

    assert len(read_stream(_CONNECTOR, _STREAM, config).records) == 1
