from __future__ import annotations

from pathlib import Path

from connector_tests.source import load_manifest

_CONNECTOR = "task-tracking/youtrack"
_EXPECTED = {
    "youtrack_projects",
    "youtrack_users",
    "youtrack_custom_fields",
    "youtrack_project_fields",
    "youtrack_field_values",
    "youtrack_agiles",
    "youtrack_sprints",
    "youtrack_issue_keys",
    "youtrack_issues",
    "youtrack_activities",
    "youtrack_work_items",
    "youtrack_comments",
    "youtrack_issue_links",
    "youtrack_issue_sprints",
    "youtrack_issue_census",
}


def test_stream_contract_and_promotions_match() -> None:
    manifest = load_manifest(_CONNECTOR)
    names = {stream["name"] for stream in manifest["streams"]}
    promotion = Path(__file__).parents[1] / "dbt" / "youtrack__bronze_promoted.sql"
    sql = promotion.read_text()

    assert names == _EXPECTED
    assert set(manifest["metadata"]["autoImportSchema"]) == _EXPECTED
    assert all(f"bronze_youtrack.{name}" in sql for name in _EXPECTED)


def test_every_stream_has_stable_bronze_stamps() -> None:
    missing = []
    for stream in load_manifest(_CONNECTOR)["streams"]:
        fields = {
            field["path"][0]
            for transform in stream.get("transformations", [])
            if transform["type"] == "AddFields"
            for field in transform["fields"]
        }
        if not {"tenant_id", "source_id", "unique_key"} <= fields:
            missing.append(stream["name"])

    assert not missing


def test_every_incremental_cursor_is_declared_in_the_stream_schema() -> None:
    missing = []
    for stream in load_manifest(_CONNECTOR)["streams"]:
        incremental = stream.get("incremental_sync")
        if not incremental:
            continue

        cursor = incremental["cursor_field"]
        properties = stream["schema_loader"]["schema"]["properties"]
        if cursor not in properties:
            missing.append((stream["name"], cursor))

    assert not missing


def test_incremental_issue_queries_keep_youtrack_datetime_syntax() -> None:
    manifest = load_manifest(_CONNECTOR)
    queries = []
    for stream in manifest["streams"]:
        requester = stream["retriever"]["requester"]
        query = requester.get("request_parameters", {}).get("query")
        if query:
            queries.append(query)

    assert queries
    assert all("updated: {{ stream_slice.start_time }} .. {{ stream_slice.end_time }}" in query for query in queries)
    assert all("sort by: updated asc" in query for query in queries)
