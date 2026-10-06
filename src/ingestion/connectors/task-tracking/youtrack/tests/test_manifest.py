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
    "youtrack_activity_issues",
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
        # youtrack_activity_issues searches by issue id, not by a time window.
        if stream.get("incremental_sync", {}).get("cursor_field") != "updated":
            continue
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
    # Only issues whose membership may have changed are read: the changed-issue
    # search plus the sprint activity feed, since an assignment leaves `updated`
    # untouched. Never the whole issue list.
    changed, sprint_activity = stream["retriever"]["partition_router"]["parent_stream_configs"]
    assert changed["stream"]["retriever"]["requester"]["request_parameters"]["query"].startswith("updated: ")
    assert sprint_activity["stream"]["retriever"]["requester"]["url"].endswith("/api/activitiesPage")
    assert sprint_activity["stream"]["retriever"]["requester"]["request_parameters"]["categories"] == "SprintCategory"
    assert changed["incremental_dependency"] and sprint_activity["incremental_dependency"]
    assert stream["incremental_sync"]["cursor_field"] == "issue_updated"


def test_activity_issues_snapshot_what_the_feed_names_in_the_issues_shape() -> None:
    manifest = load_manifest(_CONNECTOR)
    streams = {stream["name"]: stream for stream in manifest["streams"]}
    issues, activity_issues = streams["youtrack_issues"], streams["youtrack_activity_issues"]

    # Same payload and stamps, so youtrack__issues can take the latest of either.
    assert activity_issues["retriever"]["requester"]["request_parameters"]["fields"] == (
        issues["retriever"]["requester"]["request_parameters"]["fields"]
    )
    assert activity_issues["transformations"] == issues["transformations"]
    assert activity_issues["schema_loader"] == issues["schema_loader"]
    # Batched by idReadable — the search does not accept database ids.
    query = activity_issues["retriever"]["requester"]["request_parameters"]["query"]
    assert query == "issue id: {{ stream_partition.issue_id_readable | join(', ') }}"
    router = activity_issues["retriever"]["partition_router"]
    assert router["type"] == "GroupingPartitionRouter" and router["deduplicate"]
    # One batch fits the default page, so a batch is one request.
    assert router["group_size"] <= 100
    (parent,) = router["underlying_partition_router"]["parent_stream_configs"]
    assert parent["incremental_dependency"]
    parent_request = parent["stream"]["retriever"]["requester"]
    assert parent_request["url"].endswith("/api/activitiesPage")
    assert parent_request["request_parameters"]["categories"] == (
        streams["youtrack_activities"]["retriever"]["requester"]["request_parameters"]["categories"]
    )
    # The cursor cannot be `updated`: these issues are the ones it predates.
    assert activity_issues["incremental_sync"]["cursor_field"] == "observed_at"


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


def test_activity_feeds_leave_out_article_categories() -> None:
    """Silver reads Issue targets only, and the server can cut off a page that
    holds an article attachment after the response has started."""
    streams = {stream["name"]: stream for stream in load_manifest(_CONNECTOR)["streams"]}
    categories = streams["youtrack_activities"]["retriever"]["requester"]["request_parameters"]["categories"]

    assert not [c for c in categories.split(",") if c.startswith("Article")]
