"""Mock-server tests for the `jira_fields` stream.

Root-array lookup: GET /rest/api/3/field, no pagination. Flattens the nested
schema object into schema_type / schema_items / schema_custom — the columns
the dbt story-points resolution reads.

Coverage matrix rows: full_refresh_read, schema_flattening_and_stamping.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import ANY_QUERY_PARAMS, HttpMocker, HttpRequest, HttpResponse, read_stream

_STREAM = "jira_fields"
_CONNECTOR = "task-tracking/jira"
_URL = f"{JIRA_URL}/rest/api/3/field"


def _fields_response() -> HttpResponse:
    return HttpResponse(
        body=json.dumps(
            [
                {"id": "summary", "name": "Summary", "custom": False, "schema": {"type": "string"}},
                {
                    "id": "customfield_10101",
                    "name": "Story Points",
                    "custom": True,
                    "schema": {"type": "number", "custom": "com.pyxis.greenhopper.jira:jsw-story-points"},
                },
                {
                    "id": "customfield_10202",
                    "name": "Sprints",
                    "custom": True,
                    "schema": {"type": "array", "items": "string", "custom": "com.example.jira:sprint"},
                },
            ]
        ),
        status_code=200,
    )


def test_full_refresh_read(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _fields_response())

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 3
    assert not output.errors


def test_schema_flattening_and_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    http_mocker.get(HttpRequest(_URL, query_params=ANY_QUERY_PARAMS), _fields_response())

    output = read_stream(_CONNECTOR, _STREAM, config)

    by_id = {r.record.data["field_id"]: r.record.data for r in output.records}
    sp = by_id["customfield_10101"]
    assert sp["tenant_id"] == config["insight_tenant_id"]
    assert sp["source_id"] == config["insight_source_id"]
    assert sp["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-customfield_10101")
    assert sp["custom"] is True
    assert sp["schema_type"] == "number"
    # The dbt story-points resolution keys on this canonical marker.
    assert sp["schema_custom"] == "com.pyxis.greenhopper.jira:jsw-story-points"
    # None-valued AddFields stamps are dropped from the record entirely.
    assert by_id["summary"].get("schema_custom") in (None, "")
    # An array field carries the element type the flattened contract declares.
    assert by_id["customfield_10202"]["schema_type"] == "array"
    assert by_id["customfield_10202"]["schema_items"] == "string"
