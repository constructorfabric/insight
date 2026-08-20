"""Mock-server tests for the `jira_project_visibility` stream.

Full-refresh census of the projects the account can browse, partitioned over
project lifecycle status (live / archived / deleted) via ListPartitionRouter;
each partition injects `status=<s>` into GET /rest/api/3/project/search and
stamps `project_status` from the partition. See specs/DELETION-AND-VISIBILITY.md.

Coverage matrix rows: partition_fan_out, tenant_source_stamping,
schema_conformance, empty_page.
"""

from __future__ import annotations

import json

from config import JIRA_URL, JiraConfigBuilder
from connector_tests import HttpMocker, HttpRequest, HttpResponse, assert_records_conform, load_fixture, read_stream

_STREAM = "jira_project_visibility"
_CONNECTOR = "task-tracking/jira"
_SEARCH_URL = f"{JIRA_URL}/rest/api/3/project/search"


def _project(pid: int, key: str) -> dict[str, object]:
    return load_fixture(
        __file__,
        "project.json",
        id=str(pid),
        key=key,
        name=f"Project {key}",
        self=f"{JIRA_URL}/rest/api/3/project/{pid}",
    )


def _page(records: list[dict[str, object]]) -> HttpResponse:
    return HttpResponse(body=json.dumps({"values": records, "isLast": True}), status_code=200)


def _mock_status(http_mocker: HttpMocker, status: str, records: list[dict[str, object]]) -> None:
    http_mocker.get(HttpRequest(_SEARCH_URL, query_params={"status": status, "maxResults": "50"}), _page(records))


def test_partition_fan_out(http_mocker: HttpMocker) -> None:
    """One request per lifecycle status; project_status is stamped from the
    partition, so the same endpoint yields distinguishable rows."""
    config = JiraConfigBuilder().build()
    _mock_status(http_mocker, "live", [_project(10001, "LIVE1")])
    _mock_status(http_mocker, "archived", [_project(10002, "ARCH1")])
    _mock_status(http_mocker, "deleted", [_project(10003, "TRASH1")])

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 3
    assert not output.errors
    by_status = {r.record.data["project_status"]: r.record.data["project_key"] for r in output.records}
    assert by_status == {"live": "LIVE1", "archived": "ARCH1", "deleted": "TRASH1"}


def test_tenant_source_stamping(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_status(http_mocker, "live", [_project(10001, "PROJ1")])
    _mock_status(http_mocker, "archived", [])
    _mock_status(http_mocker, "deleted", [])

    output = read_stream(_CONNECTOR, _STREAM, config)

    rec = output.records[0].record.data
    assert rec["tenant_id"] == config["insight_tenant_id"]
    assert rec["source_id"] == config["insight_source_id"]
    # Keyed by immutable numeric project id (project keys can be renamed).
    assert rec["unique_key"] == (f"{config['insight_tenant_id']}-{config['insight_source_id']}-10001")
    assert rec["project_key"] == "PROJ1"


def test_schema_conformance(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    _mock_status(http_mocker, "live", [_project(10001, "PROJ1")])
    _mock_status(http_mocker, "archived", [_project(10002, "ARCH1")])
    _mock_status(http_mocker, "deleted", [])

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert_records_conform(output.records, _CONNECTOR, _STREAM)


def test_empty_page(http_mocker: HttpMocker) -> None:
    config = JiraConfigBuilder().build()
    for status in ("live", "archived", "deleted"):
        _mock_status(http_mocker, status, [])

    output = read_stream(_CONNECTOR, _STREAM, config)

    assert len(output.records) == 0
    assert not output.errors
