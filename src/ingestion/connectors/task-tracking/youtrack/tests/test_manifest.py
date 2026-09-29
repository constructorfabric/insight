from __future__ import annotations

import pytest
from connector_tests.source import load_manifest
from jsonschema import ValidationError, validate

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


def test_the_manifest_declares_exactly_the_contracted_streams() -> None:
    """Bronze tables are created by the destination from this stream set, so the
    manifest and the contract cannot drift apart unnoticed."""
    manifest = load_manifest(_CONNECTOR)
    names = {stream["name"] for stream in manifest["streams"]}

    assert names == _EXPECTED
    assert set(manifest["metadata"]["autoImportSchema"]) == _EXPECTED


def test_every_stream_has_stable_bronze_stamps() -> None:
    missing = []
    for stream in load_manifest(_CONNECTOR)["streams"]:
        fields = set()
        for transform in stream.get("transformations", []):
            if transform["type"] != "AddFields":
                continue

            for field in transform["fields"]:
                fields.add(field["path"][0])
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


def test_project_request_excludes_paginated_child_collections() -> None:
    manifest = load_manifest(_CONNECTOR)
    projects = next(stream for stream in manifest["streams"] if stream["name"] == "youtrack_projects")
    fields = projects["retriever"]["requester"]["request_parameters"]["fields"]

    assert fields == (
        "id,archived,createdBy(id,login,fullName,email),"
        "description,fromEmail,iconUrl,"
        "leader(id,login,fullName,email),name,replyToEmail,shortName,"
        "team(id,name,ringId,usersCount,icon,allUsersGroup),template"
    )


def test_bundle_values_use_all_paginated_resource_collections() -> None:
    manifest = load_manifest(_CONNECTOR)
    streams = {stream["name"]: stream for stream in manifest["streams"]}
    field_values = streams["youtrack_field_values"]
    configs = field_values["retriever"]["partition_router"]["parent_stream_configs"]

    assert {config["partition_field"] for config in configs} == {
        "values_bundle_id",
        "user_groups_bundle_id",
        "user_individuals_bundle_id",
        "user_aggregated_bundle_id",
    }
    assert field_values["retriever"]["paginator"]["page_token_option"]["field_name"] == "$skip"
    value_fields = field_values["retriever"]["requester"]["request_parameters"]["fields"]
    assert "localizedName" in value_fields
    assert "isResolved" in value_fields
    assert "startDate" in value_fields
    assert "assembleDate" in value_fields
    assert "owner(id,login,fullName,email)" in value_fields
    project_fields = streams["youtrack_project_fields"]
    project_field_request = project_fields["retriever"]["requester"]["request_parameters"]["fields"]
    assert "bundle(id,$type,name)" in project_field_request
    assert "bundle(id,$type,name,values(" not in project_field_request


def test_issue_sprint_membership_is_paginated_from_issues() -> None:
    manifest = load_manifest(_CONNECTOR)
    stream = next(stream for stream in manifest["streams"] if stream["name"] == "youtrack_issue_sprints")

    assert stream["retriever"]["requester"]["url"].endswith("/api/issues/{{ stream_partition.issue_id }}/sprints")
    assert stream["retriever"]["paginator"]["page_token_option"]["field_name"] == "$skip"


def test_work_items_are_full_refresh_and_allow_null_updated() -> None:
    manifest = load_manifest(_CONNECTOR)
    stream = next(stream for stream in manifest["streams"] if stream["name"] == "youtrack_work_items")

    assert "incremental_sync" not in stream
    assert "partition_router" not in stream["retriever"]
    assert stream["retriever"]["requester"]["url"].endswith("/api/workItems")
    assert "null" in stream["schema_loader"]["schema"]["properties"]["updated"]["type"]


def test_comments_allow_null_updated() -> None:
    manifest = load_manifest(_CONNECTOR)
    stream = next(stream for stream in manifest["streams"] if stream["name"] == "youtrack_comments")

    assert "null" in stream["schema_loader"]["schema"]["properties"]["updated"]["type"]


def test_start_date_is_required_without_a_default() -> None:
    manifest = load_manifest(_CONNECTOR)
    specification = manifest["spec"]["connection_specification"]
    config_without_start_date = {
        "insight_tenant_id": "test-tenant",
        "insight_source_id": "test-source",
        "youtrack_base_url": "https://example.youtrack.invalid",
        "youtrack_token": "synthetic-token",
    }

    assert "youtrack_start_date" in specification["required"]
    assert "default" not in specification["properties"]["youtrack_start_date"]
    with pytest.raises(ValidationError):
        validate(instance=config_without_start_date, schema=specification)


def test_sprint_requests_exclude_embedded_issues() -> None:
    manifest = load_manifest(_CONNECTOR)
    stream = next(stream for stream in manifest["streams"] if stream["name"] == "youtrack_sprints")
    fields = stream["retriever"]["requester"]["request_parameters"]["fields"]

    assert "issues(" not in fields
    assert "unresolvedIssuesCount" in fields
