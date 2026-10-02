from __future__ import annotations

import json

import freezegun
from config import API_URL, YouTrackConfigBuilder
from connector_tests import (
    ANY_QUERY_PARAMS,
    HttpMocker,
    HttpRequest,
    HttpResponse,
    assert_records_conform,
    load_fixture,
    read_stream,
)
from connector_tests.source import load_manifest

_CONNECTOR = "task-tracking/youtrack"
_NOW = "2026-07-01T00:00:00Z"


def test_projects_are_paginated_stamped_and_schema_valid(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    project = load_fixture(__file__, "project.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_projects", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["unique_key"] == "test-tenant-test-source-0-1"
    assert json.loads(record["project_json"])["shortName"] == "EX"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_projects")


def test_custom_field_snapshot_preserves_cardinality_and_json(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    field = load_fixture(__file__, "custom_field.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/customFieldSettings/customFields", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([field]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_custom_fields", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["field_type_id"] == "version[*]"
    assert json.loads(record["metadata_json"])["fieldType"]["id"] == "version[*]"


@freezegun.freeze_time(_NOW)
def test_field_values_paginate_the_bundle_endpoint(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    project = load_fixture(__file__, "project.json")
    project_field = load_fixture(__file__, "project_field.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects/0-1/customFields", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project_field]), status_code=200),
    )
    value = project_field["bundle"]["values"][0]
    first_page = [{**value, "id": f"133-{index}"} for index in range(100)]
    last_value = {**value, "id": "133-100"}
    value_fields = (
        "id,$type,name,description,archived,ordinal,color(id,background,foreground),"
        "hasRunningJob,released,releaseDate,startDate"
    )
    http_mocker.get(
        HttpRequest(
            f"{API_URL}/admin/customFieldSettings/bundles/version/72-1/values",
            query_params={"fields": value_fields, "$top": "100"},
        ),
        HttpResponse(body=json.dumps(first_page), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{API_URL}/admin/customFieldSettings/bundles/version/72-1/values",
            query_params={"fields": value_fields, "$top": "100", "$skip": "100"},
        ),
        HttpResponse(body=json.dumps([last_value]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_field_values", config)

    assert not output.errors
    assert len(output.records) == 101
    record = output.records[-1].record.data
    assert record["field_id"] == "58-1"
    assert record["field_type_id"] == "version[*]"
    assert record["bundle_id"] == "72-1"
    assert json.loads(record["bundle_json"])["id"] == "72-1"
    value_payload = json.loads(record["values_json"])
    assert value_payload["collection"] == "values"
    assert value_payload["value"]["id"] == "133-100"
    assert len({item.record.data["observed_at"] for item in output.records}) == 1
    assert_records_conform(output.records, _CONNECTOR, "youtrack_field_values")


@freezegun.freeze_time(_NOW)
def test_user_bundle_values_read_all_distinct_resources(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    project = load_fixture(__file__, "project.json")
    project_field = {
        "id": "42-2",
        "$type": "UserProjectCustomField",
        "field": {"id": "58-2", "name": "Assignees", "fieldType": {"id": "user[*]"}},
        "bundle": {"id": "72-2", "$type": "UserBundle", "name": "Example users"},
    }
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/admin/projects/0-1/customFields", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([project_field]), status_code=200),
    )
    resources = {
        "groups": {"id": "3-1", "$type": "UserGroup", "name": "Example Group"},
        "individuals": {"id": "1-1", "$type": "User", "login": "example-user"},
        "aggregatedUsers": {"id": "1-2", "$type": "User", "login": "group-user"},
    }
    for resource, value in resources.items():
        fields = (
            "id,name,ringId,$type"
            if resource == "groups"
            else "id,login,fullName,email,ringId,guest,online,banned,banBadge,banReason,isAnonymized,$type"
        )
        http_mocker.get(
            HttpRequest(
                f"{API_URL}/admin/customFieldSettings/bundles/user/72-2/{resource}",
                query_params={"fields": fields, "$top": "100"},
            ),
            HttpResponse(body=json.dumps([value]), status_code=200),
        )

    output = read_stream(_CONNECTOR, "youtrack_field_values", config)

    assert not output.errors
    collections = {json.loads(item.record.data["values_json"])["collection"] for item in output.records}
    assert collections == set(resources)
    assert all(item.record.data["field_type_id"] == "user[*]" for item in output.records)
    assert all(json.loads(item.record.data["bundle_json"])["id"] == "72-2" for item in output.records)
    assert_records_conform(output.records, _CONNECTOR, "youtrack_field_values")


@freezegun.freeze_time(_NOW)
def test_issue_payload_keeps_custom_fields_as_json(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    issue = load_fixture(__file__, "issue.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([issue]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_issues", config)

    assert not output.errors
    record = output.records[0].record.data
    assert "customFields" not in record
    assert json.loads(record["custom_fields_json"])[0]["$type"] == "MultiVersionIssueCustomField"
    assert json.loads(record["issue_json"])["idReadable"] == "EX-1"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_issues")


@freezegun.freeze_time(_NOW)
def test_activity_issues_batch_the_feed_targets_into_one_search(http_mocker: HttpMocker) -> None:
    # The feed names EX-1 twice (one search term), an article and an idReadable-
    # less draft (no term); the issue's `updated` predates the window and the
    # snapshot is taken anyway.
    config = YouTrackConfigBuilder().build()
    issue = {**load_fixture(__file__, "issue.json"), "updated": 1782691200000}
    feed = {
        "afterCursor": None,
        "hasAfter": False,
        "activities": [
            {"id": "2-1.0-1", "timestamp": 1782820800000, "target": {"id": "2-1", "idReadable": "EX-1", "$type": "Issue"}},
            {"id": "2-1.0-2", "timestamp": 1782824400000, "target": {"id": "2-1", "idReadable": "EX-1", "$type": "Issue"}},
            {"id": "9-1.0-1", "timestamp": 1782824400000, "target": {"id": "9-1", "$type": "Article"}},
            {"id": "2-9.0-1", "timestamp": 1782824400000, "target": {"id": "2-9", "$type": "Issue"}},
        ],
    }
    http_mocker.get(
        HttpRequest(f"{API_URL}/activitiesPage", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps(feed), status_code=200),
    )
    streams = {stream["name"]: stream for stream in load_manifest(_CONNECTOR)["streams"]}
    fields = streams["youtrack_activity_issues"]["retriever"]["requester"]["request_parameters"]["fields"]
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues", query_params={"fields": fields, "query": "issue id: EX-1", "$top": "100"}),
        HttpResponse(body=json.dumps([issue]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_activity_issues", config)

    assert not output.errors
    assert [item.record.data["id"] for item in output.records] == ["2-1"]
    record = output.records[0].record.data
    assert record["unique_key"] == "test-tenant-test-source-2-1"
    assert json.loads(record["issue_json"])["idReadable"] == "EX-1"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_activity_issues")


@freezegun.freeze_time(_NOW)
def test_activity_cursor_paginates_and_preserves_polymorphic_values(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    first = load_fixture(__file__, "activity_page.json")
    second = {"afterCursor": None, "hasAfter": False, "activities": []}
    http_mocker.get(
        HttpRequest(f"{API_URL}/activitiesPage", query_params=ANY_QUERY_PARAMS),
        [HttpResponse(body=json.dumps(first), status_code=200), HttpResponse(body=json.dumps(second), status_code=200)],
    )

    output = read_stream(_CONNECTOR, "youtrack_activities", config)

    assert not output.errors
    record = output.records[0].record.data
    assert record["unique_key"] == "test-tenant-test-source-2-1.0-1"
    assert json.loads(record["field_json"])["$type"] == "CustomFilterField"
    assert json.loads(record["added_json"])[0]["name"] == "Example Version"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_activities")


@freezegun.freeze_time(_NOW)
def test_issue_sprints_paginate_the_issue_membership_endpoint(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().with_field("youtrack_page_size", "1").build()
    issue = load_fixture(__file__, "issue.json")
    sprint = load_fixture(__file__, "sprint.json")
    # The changed-issue parent yields the issue once; every later window page is empty.
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues", query_params=ANY_QUERY_PARAMS),
        [HttpResponse(body=json.dumps([issue]), status_code=200), HttpResponse(body=json.dumps([]), status_code=200)],
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/activitiesPage", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps({"afterCursor": None, "hasAfter": False, "activities": []}), status_code=200),
    )
    next_sprint = {**sprint, "id": "120-2", "name": "Next Sprint"}
    http_mocker.get(
        HttpRequest(
            f"{API_URL}/issues/2-1/sprints",
            query_params={
                "fields": "id,name,goal,start,finish,archived,isDefault,unresolvedIssuesCount,agile(id,name)",
                "$top": "1",
            },
        ),
        HttpResponse(body=json.dumps([sprint]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{API_URL}/issues/2-1/sprints",
            query_params={
                "fields": "id,name,goal,start,finish,archived,isDefault,unresolvedIssuesCount,agile(id,name)",
                "$top": "1",
                "$skip": "1",
            },
        ),
        HttpResponse(body=json.dumps([next_sprint]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(
            f"{API_URL}/issues/2-1/sprints",
            query_params={
                "fields": "id,name,goal,start,finish,archived,isDefault,unresolvedIssuesCount,agile(id,name)",
                "$top": "1",
                "$skip": "2",
            },
        ),
        HttpResponse(body=json.dumps([]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_issue_sprints", config)

    assert not output.errors
    assert [item.record.data["sprint_id"] for item in output.records] == ["120-1", "120-2"]
    assert all(item.record.data["issue_id"] == "2-1" for item in output.records)
    first_record = output.records[0].record.data
    assert set(first_record) == {
        "issue_id",
        "issue_id_readable",
        "issue_updated",
        "source_id",
        "sprint_id",
        "sprint_json",
        "tenant_id",
        "unique_key",
    }
    assert json.loads(first_record["sprint_json"])["goal"] == "Synthetic sprint goal"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_issue_sprints")


@freezegun.freeze_time(_NOW)
def test_issue_sprints_follow_sprint_activity_without_an_updated_bump(http_mocker: HttpMocker) -> None:
    # A sprint assignment leaves the issue's `updated` untouched, so only the
    # sprint activity feed names the issue; non-issue targets yield no partition.
    config = YouTrackConfigBuilder().build()
    sprint = load_fixture(__file__, "sprint.json")
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([]), status_code=200),
    )
    activities = {
        "afterCursor": None,
        "hasAfter": False,
        "activities": [
            {
                "id": "2-7.0-1",
                "$type": "SprintActivityItem",
                "timestamp": 1782820800000,
                "target": {"id": "2-7", "idReadable": "EX-7", "$type": "Issue"},
            },
            {
                "id": "9-1.0-1",
                "$type": "SprintActivityItem",
                "timestamp": 1782820800000,
                "target": {"id": "9-1", "$type": "Article"},
            },
        ],
    }
    http_mocker.get(
        HttpRequest(f"{API_URL}/activitiesPage", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps(activities), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues/2-7/sprints", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([sprint]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_issue_sprints", config)

    assert not output.errors
    records = {(item.record.data["issue_id"], item.record.data["sprint_id"]) for item in output.records}
    assert records == {("2-7", "120-1")}
    record = output.records[0].record.data
    assert record["issue_id_readable"] == "EX-7"
    assert str(record["issue_updated"]) == "1782820800000"
    assert_records_conform(output.records, _CONNECTOR, "youtrack_issue_sprints")


def test_work_items_keep_never_updated_records(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    work_item = {
        "id": "91-1",
        "date": 1782777600000,
        "created": 1782777600000,
        "updated": None,
        "issue": {"id": "2-1", "idReadable": "EX-1"},
        "author": {"id": "1-1", "login": "example"},
    }
    http_mocker.get(
        HttpRequest(f"{API_URL}/workItems", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([work_item]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_work_items", config)

    assert not output.errors
    assert output.records[0].record.data["issue_id"] == "2-1"
    assert json.loads(output.records[0].record.data["work_item_json"])["updated"] is None
    assert_records_conform(output.records, _CONNECTOR, "youtrack_work_items")


@freezegun.freeze_time(_NOW)
def test_comments_allow_never_updated_records(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    issue = load_fixture(__file__, "issue.json")
    comment = {
        "id": "4-1",
        "text": "Synthetic comment",
        "created": 1782777600000,
        "updated": None,
        "deleted": False,
        "author": {"id": "1-1", "login": "example"},
    }
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([issue]), status_code=200),
    )
    http_mocker.get(
        HttpRequest(f"{API_URL}/issues/2-1/comments", query_params=ANY_QUERY_PARAMS),
        HttpResponse(body=json.dumps([comment]), status_code=200),
    )

    output = read_stream(_CONNECTOR, "youtrack_comments", config)

    assert not output.errors
    assert json.loads(output.records[0].record.data["comment_json"])["updated"] is None
    assert_records_conform(output.records, _CONNECTOR, "youtrack_comments")


def test_work_items_paginate_without_per_issue_requests(http_mocker: HttpMocker) -> None:
    config = YouTrackConfigBuilder().build()
    config["youtrack_page_size"] = "2"
    manifest = load_manifest(_CONNECTOR)
    stream = next(stream for stream in manifest["streams"] if stream["name"] == "youtrack_work_items")
    fields = stream["retriever"]["requester"]["request_parameters"]["fields"]
    records = [{"id": f"91-{index}", "updated": None, "issue": {"id": f"2-{index}"}} for index in range(3)]
    for offset, page in [(0, records[:2]), (2, records[2:])]:
        params = {"fields": fields, "$top": "2"}
        if offset:
            params["$skip"] = str(offset)
        http_mocker.get(
            HttpRequest(f"{API_URL}/workItems", query_params=params),
            HttpResponse(body=json.dumps(page), status_code=200),
        )

    output = read_stream(_CONNECTOR, "youtrack_work_items", config)

    assert not output.errors
    assert {record.record.data["issue_id"] for record in output.records} == {"2-0", "2-1", "2-2"}
    assert len(http_mocker._mocker.request_history) == 2
    assert_records_conform(output.records, _CONNECTOR, "youtrack_work_items")
