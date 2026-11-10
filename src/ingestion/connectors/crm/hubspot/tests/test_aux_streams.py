"""Pipelines, property definitions and property history streams."""

from __future__ import annotations

import json

import pytest
from source_hubspot.constants import BASE_URL, HISTORY_BATCH_READ_LIMIT, PROPERTY_DEFINITION_OBJECT_TYPES
from source_hubspot.streams import PipelinesStream, PropertyDefinitionsStream, PropertyHistoryStream
from tests.conftest import SOURCE, TENANT, FakeResponse, make_stream, wire


def change(value, ts="2024-06-01T00:00:00Z", source_type="CRM_UI", user=7):
    return {"value": value, "timestamp": ts, "sourceType": source_type, "sourceId": "u", "updatedByUserId": user}


class TestPipelines:
    def test_emits_every_pipeline_tagged_with_its_object_type(self):
        stream = make_stream(PipelinesStream, "ticket_pipelines")
        client = wire(stream, [FakeResponse({"results": [{"id": "0", "label": "Support", "stages": [{"id": "1"}]}]})])

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
        pages = [FakeResponse({"results": []}) for _ in range(2 * len(PROPERTY_DEFINITION_OBJECT_TYPES))]
        pages[0] = FakeResponse({"results": [{"name": "amount", "label": "Amount"}]})
        pages[1] = FakeResponse({"results": [{"name": "old_field", "label": "Old"}]})
        client = wire(stream, pages)

        out = list(stream.read_records(None))

        first = PROPERTY_DEFINITION_OBJECT_TYPES[0]
        assert [r["id"] for r in out] == [f"{first}:amount", f"{first}:old_field"]
        assert [r["archived"] for r in out] == [False, True]
        assert client.calls[0]["params"] == {"archived": "false"}
        assert client.calls[1]["params"] == {"archived": "true"}

    def test_same_name_on_two_objects_gets_two_keys(self):
        stream = make_stream(PropertyDefinitionsStream, "property_definitions")
        pages = [
            FakeResponse({"results": [{"name": "hubspot_owner_id"}]})
            for _ in range(2 * len(PROPERTY_DEFINITION_OBJECT_TYPES))
        ]
        wire(stream, pages)

        keys = {r["unique_key"] for r in stream.read_records(None)}

        assert len(keys) == len(PROPERTY_DEFINITION_OBJECT_TYPES), "keys must be unique per (object, name)"

    def test_nameless_definition_is_skipped(self):
        stream = make_stream(PropertyDefinitionsStream, "property_definitions")
        pages = [
            FakeResponse({"results": [{"label": "no name"}]}) for _ in range(2 * len(PROPERTY_DEFINITION_OBJECT_TYPES))
        ]
        wire(stream, pages)
        assert list(stream.read_records(None)) == []


class TestPropertyHistory:
    def history_result(self, object_id, updated="2024-06-02T00:00:00Z", **history):
        return {"id": object_id, "updatedAt": updated, "propertiesWithHistory": history}

    def test_search_requests_only_object_id(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        client = wire(stream, [FakeResponse({"results": []})])

        list(stream.read_records(None))

        assert client.calls[0]["json"]["properties"] == ["hs_object_id"]

    def test_one_record_per_change(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        client = wire(
            stream,
            [
                FakeResponse({"results": [{"id": "10", "updatedAt": "2024-06-02T00:00:00Z"}]}),
                FakeResponse(
                    {
                        "results": [
                            self.history_result(
                                "10",
                                dealstage=[change("won"), change("open", ts="2024-05-01T00:00:00Z")],
                                amount=[change("5")],
                            )
                        ]
                    }
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
        client = wire(
            stream, [FakeResponse({"results": ids}), FakeResponse({"results": []}), FakeResponse({"results": []})]
        )

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
        wire(
            stream,
            [
                FakeResponse({"results": [{"id": "10"}]}),
                FakeResponse({"results": [result]}),
                FakeResponse({"results": [{"id": "10"}]}),
                FakeResponse({"results": [twin]}),
            ],
        )

        first = list(stream.read_records(None))
        replay = list(stream.read_records(None))

        assert first[0]["unique_key"] == replay[0]["unique_key"], "a replayed change must keep its key"
        assert replay[0]["unique_key"] != replay[1]["unique_key"], f"distinct change shares a key: {other!r}"

    def test_raw_data_carries_the_change(self):
        stream = make_stream(PropertyHistoryStream, "deals_property_history")
        wire(
            stream,
            [
                FakeResponse({"results": [{"id": "10"}]}),
                FakeResponse({"results": [self.history_result("10", pipeline=[change("default")])]}),
            ],
        )

        out = list(stream.read_records(None))

        assert json.loads(out[0]["raw_data"])["value"] == "default"
