"""Pipelines, property definitions and property history streams."""

from __future__ import annotations

import hashlib
import json

import pytest
from airbyte_cdk.models import FailureType
from airbyte_cdk.utils import AirbyteTracedException
from source_hubspot.constants import (
    BASE_URL,
    HISTORY_BATCH_READ_LIMIT,
    MISSING_CURSOR_TIMESTAMP,
    PROPERTY_DEFINITION_OBJECT_TYPES,
)
from source_hubspot.models import HistoryObject, HistoryPage, PropertyChange, RecordPage
from source_hubspot.streams import PipelinesStream, PropertyDefinitionsStream, PropertyHistoryStream
from tests.conftest import SOURCE, TENANT, FakeResponse, assert_no_leak, make_stream, wire

PERSONAL = "person@example.com"


def change(value, ts="2024-06-01T00:00:00Z", source_type="CRM_UI", user=7):
    return PropertyChange(value=value, timestamp=ts, sourceType=source_type, sourceId="u", updatedByUserId=user)


def page(records):
    return FakeResponse(RecordPage(results=records).model_dump(mode="json"))


def history_page(objects):
    return FakeResponse(HistoryPage(results=objects).model_dump(mode="json"))


def definition_pages(*results):
    pages = [page([]) for _ in range(2 * len(PROPERTY_DEFINITION_OBJECT_TYPES))]
    pages[0] = page(list(results))
    return pages


@pytest.mark.parametrize(
    ("stream_cls", "name", "pages"),
    [
        (PipelinesStream, "deal_pipelines", lambda results: [page(results)]),
        (PropertyDefinitionsStream, "property_definitions", lambda results: definition_pages(*results)),
    ],
)
@pytest.mark.parametrize(
    ("record", "expected"),
    [
        (
            {"id": "a", "name": "a", "updatedAt": "2024-06-02T00:00:00Z", "createdAt": "2024-01-01T00:00:00Z"},
            "2024-06-02T00:00:00Z",
        ),
        ({"id": "b", "name": "b", "createdAt": "2024-01-01T00:00:00Z"}, "2024-01-01T00:00:00Z"),
        ({"id": "c", "name": "c", "updatedAt": None}, MISSING_CURSOR_TIMESTAMP),
        ({"id": "d", "name": "d"}, MISSING_CURSOR_TIMESTAMP),
    ],
)
def test_every_record_carries_a_cursor_the_destination_can_store(stream_cls, name, pages, record, expected):
    stream = make_stream(stream_cls, name)
    wire(stream, pages([record]))

    out = list(stream.read_records(None))

    assert [r["updatedAt"] for r in out] == [expected], f"cursor for {record!r}"


class TestPipelines:
    def test_emits_every_pipeline_tagged_with_its_object_type(self):
        stream = make_stream(PipelinesStream, "ticket_pipelines")
        client = wire(stream, [page([{"id": "0", "label": "Support", "stages": [{"id": "1"}]}])])

        out = list(stream.read_records(None))

        assert client.calls[0]["url"] == f"{BASE_URL}/crm/v3/pipelines/tickets"
        assert out[0]["objectType"] == "tickets"
        assert out[0]["stages"] == [{"id": "1"}]
        assert out[0]["unique_key"] == f"{TENANT}-{SOURCE}-0"

    def test_schema_declares_stages(self):
        props = make_stream(PipelinesStream, "deal_pipelines").get_json_schema()["properties"]
        assert props["stages"]["type"] == ["array", "null"]
        assert "raw_data" in props


class TestPropertyDefinitions:
    def test_lists_live_and_archived_for_every_object_type(self):
        stream = make_stream(PropertyDefinitionsStream, "property_definitions")
        pages = [page([]) for _ in range(2 * len(PROPERTY_DEFINITION_OBJECT_TYPES))]
        pages[0] = page([{"name": "amount", "label": "Amount"}])
        pages[1] = page([{"name": "old_field", "label": "Old"}])
        client = wire(stream, pages)

        out = list(stream.read_records(None))

        first = PROPERTY_DEFINITION_OBJECT_TYPES[0]
        assert [r["id"] for r in out] == [f"{first}:amount", f"{first}:old_field"]
        assert [r["archived"] for r in out] == [False, True]
        assert client.calls[0]["params"] == {"archived": "false"}
        assert client.calls[1]["params"] == {"archived": "true"}

    def test_same_name_on_two_objects_gets_two_keys(self):
        stream = make_stream(PropertyDefinitionsStream, "property_definitions")
        pages = [page([{"name": "hubspot_owner_id"}]) for _ in range(2 * len(PROPERTY_DEFINITION_OBJECT_TYPES))]
        wire(stream, pages)

        keys = {r["unique_key"] for r in stream.read_records(None)}

        assert len(keys) == len(PROPERTY_DEFINITION_OBJECT_TYPES), "keys must be unique per (object, name)"

    def test_nameless_definition_is_skipped(self):
        stream = make_stream(PropertyDefinitionsStream, "property_definitions")
        pages = [page([{"label": "no name"}]) for _ in range(2 * len(PROPERTY_DEFINITION_OBJECT_TYPES))]
        wire(stream, pages)
        assert list(stream.read_records(None)) == []


class TestPropertyHistory:
    def history_result(self, object_id, updated="2024-06-02T00:00:00Z", **history):
        return HistoryObject(id=object_id, updatedAt=updated, propertiesWithHistory=history)

    def test_search_requests_only_object_id(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        client = wire(stream, [page([])])

        list(stream.read_records(None))

        assert client.calls[0]["json"]["properties"] == ["hs_object_id"]

    def test_one_record_per_change(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        client = wire(
            stream,
            [
                page([{"id": "10", "updatedAt": "2024-06-02T00:00:00Z"}]),
                history_page(
                    [
                        self.history_result(
                            "10",
                            dealstage=[change("won"), change("open", ts="2024-05-01T00:00:00Z")],
                            amount=[change("5")],
                        )
                    ]
                ),
            ],
        )

        out = list(stream.read_records(None))

        batch = client.calls[1]
        assert batch["url"] == f"{BASE_URL}/crm/v3/objects/deals/batch/read"
        assert batch["json"]["inputs"] == [{"id": "10"}]
        assert "dealstage" in batch["json"]["propertiesWithHistory"]
        assert sorted((r["propertyName"], r["value"]) for r in out) == [
            ("amount", "5"),
            ("dealstage", "open"),
            ("dealstage", "won"),
        ]
        assert {r["objectId"] for r in out} == {"10"}
        assert stream.state == {"updatedAt": "2024-06-02T00:00:00Z"}

    def test_batch_read_respects_history_limit(self):
        stream = make_stream(PropertyHistoryStream, "tickets_property_history")
        ids = [{"id": str(i), "updatedAt": "2024-06-02T00:00:00Z"} for i in range(HISTORY_BATCH_READ_LIMIT + 1)]
        client = wire(stream, [page(ids), history_page([]), history_page([])])

        list(stream.read_records(None))

        sizes = [len(c["json"]["inputs"]) for c in client.calls[1:]]
        assert sizes == [HISTORY_BATCH_READ_LIMIT, 1]

    @pytest.mark.parametrize(
        "other",
        [
            change("won", ts="2024-06-02T00:00:00Z"),
            change("lost"),
            change("won", source_type="API"),
            change("won", user=8),
        ],
    )
    def test_change_key_is_stable_and_distinct(self, other):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        result = self.history_result("10", dealstage=[change("won")])
        twin = self.history_result("10", dealstage=[change("won"), other])
        wire(stream, [page([{"id": "10"}]), history_page([result]), page([{"id": "10"}]), history_page([twin])])

        first = list(stream.read_records(None))
        replay = list(stream.read_records(None))

        assert first[0]["unique_key"] == replay[0]["unique_key"], "a replayed change must keep its key"
        assert replay[0]["unique_key"] != replay[1]["unique_key"], f"distinct change shares a key: {other!r}"

    def test_raw_data_carries_the_change(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        wire(stream, [page([{"id": "10"}]), history_page([self.history_result("10", pipeline=[change("default")])])])

        out = list(stream.read_records(None))

        assert json.loads(out[0]["raw_data"])["value"] == "default"

    def test_change_fields_reach_bronze_as_they_arrived(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        odd = {"value": {"amount": 5}, "timestamp": 1717200000000, "sourceLabel": ["a"], "updatedByUserId": "7"}
        result = {"id": "10", "updatedAt": "2024-06-02T00:00:00Z", "propertiesWithHistory": {"dealstage": [odd]}}
        wire(stream, [page([{"id": "10"}]), FakeResponse({"results": [result]})])

        out = list(stream.read_records(None))

        emitted = {key: out[0][key] for key in odd}
        assert emitted == odd
        assert json.loads(out[0]["raw_data"])["value"] == {"amount": 5}

    def test_change_key_matches_str_of_each_raw_field(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        raw = {"value": 5, "timestamp": 0, "sourceType": None, "sourceId": "u", "updatedByUserId": 7}
        result = {"id": "10", "propertiesWithHistory": {"dealstage": [raw]}}
        wire(stream, [page([{"id": "10"}]), FakeResponse({"results": [result]})])

        out = list(stream.read_records(None))

        fields = ("", "5", "", "u", "7")
        fingerprint = "\x1f".join(fields)
        digest = hashlib.sha256(fingerprint.encode("utf-8")).hexdigest()[:16]
        assert out[0]["id"] == f"10:dealstage:{digest}"

    def test_null_history_tolerated(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        nulls = {"results": [{"id": "10", "propertiesWithHistory": {"dealstage": None}}, {"id": "11"}]}
        wire(stream, [page([{"id": "10"}]), FakeResponse(nulls)])

        assert list(stream.read_records(None)) == []

    @pytest.mark.parametrize(
        "result",
        [
            {"propertiesWithHistory": {}},
            {"id": "10", "propertiesWithHistory": {"dealstage": PERSONAL}},
            {"id": "10", "propertiesWithHistory": {"dealstage": [PERSONAL]}},
            {"id": "10", "updatedAt": {"by": PERSONAL}},
        ],
    )
    def test_malformed_history_is_system_error_without_input(self, result):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        wire(stream, [page([{"id": "10"}]), FakeResponse({"results": [result]})])

        with pytest.raises(AirbyteTracedException) as exc_info:
            list(stream.read_records(None))

        error = exc_info.value
        assert error.failure_type == FailureType.system_error, f"should reject: {result!r}"
        assert_no_leak(error, PERSONAL)
