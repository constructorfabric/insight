"""CrmSearchStream: search body building, pagination, 10k-cap keyset restart."""

from __future__ import annotations

import logging

from source_hubspot.constants import BASE_URL, SEARCH_PAGE_LIMIT
from tests.conftest import SOURCE, TENANT, FakeResponse, wire


def search_page(records, next_after=None):
    payload = {"results": records}
    if next_after is not None:
        payload["paging"] = {"next": {"after": next_after}}
    return FakeResponse(payload)


def rec(i, updated="2024-06-01T00:00:00Z", **props):
    return {"id": str(i), "updatedAt": updated, "properties": dict(props)}


class TestSearchBody:
    def test_window_filters_and_sort(self, deals_stream):
        body = deals_stream._search_body(
            lower="L", upper="U", property_names=["amount"], after=None, min_object_id=None
        )
        assert body["filterGroups"] == [
            {
                "filters": [
                    {"propertyName": "hs_lastmodifieddate", "operator": "GTE", "value": "L"},
                    {"propertyName": "hs_lastmodifieddate", "operator": "LTE", "value": "U"},
                ]
            }
        ]
        assert body["sorts"] == [{"propertyName": "hs_object_id", "direction": "ASCENDING"}]
        assert body["properties"] == ["amount"]
        assert body["limit"] == SEARCH_PAGE_LIMIT
        assert "after" not in body

    def test_after_and_keyset_filter(self, deals_stream):
        body = deals_stream._search_body(lower="L", upper="U", property_names=[], after="200", min_object_id="42")
        assert body["after"] == "200"
        assert body["filterGroups"][0]["filters"][2] == {
            "propertyName": "hs_object_id",
            "operator": "GT",
            "value": "42",
        }

    def test_contacts_use_unprefixed_cursor_property(self):
        from source_hubspot.streams import CrmSearchStream
        from tests.conftest import make_stream

        contacts = make_stream(CrmSearchStream, "contacts")
        body = contacts._search_body(lower="L", upper="U", property_names=[], after=None, min_object_id=None)
        assert body["filterGroups"][0]["filters"][0]["propertyName"] == "lastmodifieddate"


class TestPostSearch:
    def test_parses_results_and_after(self, deals_stream):
        client = wire(deals_stream, [search_page([rec(1)], next_after="100")])
        results, after = deals_stream._post_search({"q": 1})
        assert [r["id"] for r in results] == ["1"]
        assert after == "100"
        call = client.calls[0]
        assert call["method"] == "POST"
        assert call["url"] == f"{BASE_URL}/crm/v3/objects/deals/search"
        assert call["json"] == {"q": 1}

    def test_missing_paging_yields_no_after(self, deals_stream):
        wire(deals_stream, [FakeResponse({"results": []})])
        results, after = deals_stream._post_search({})
        assert results == [] and after is None

    def test_malformed_paging_shapes_ignored(self, deals_stream):
        wire(
            deals_stream,
            [
                FakeResponse({"results": [rec(1)], "paging": "oops"}),
                FakeResponse({"results": [rec(1)], "paging": {"next": "oops"}}),
            ],
        )
        assert deals_stream._post_search({})[1] is None
        assert deals_stream._post_search({})[1] is None


class TestGenerateRecords:
    def test_single_page(self, deals_stream):
        client = wire(deals_stream, [search_page([rec(1), rec(2)])])
        out = list(deals_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1", "2"]
        # First sync: lower bound is the configured start_date.
        filters = client.calls[0]["json"]["filterGroups"][0]["filters"]
        assert filters[0]["value"] == "2024-01-01T00:00:00Z"
        assert filters[1]["value"] == deals_stream._init_sync.to_iso8601_string()

    def test_empty_first_page(self, deals_stream):
        wire(deals_stream, [search_page([])])
        assert list(deals_stream._generate_records(None, None, None)) == []

    def test_pagination_follows_after(self, deals_stream):
        client = wire(deals_stream, [search_page([rec(1)], next_after="100"), search_page([rec(2)])])
        out = list(deals_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1", "2"]
        assert "after" not in client.calls[0]["json"]
        assert client.calls[1]["json"]["after"] == "100"

    def test_state_sets_lower_bound(self, deals_stream):
        deals_stream.state = {"updatedAt": "2024-05-15T10:00:00Z"}
        client = wire(deals_stream, [search_page([])])
        list(deals_stream._generate_records(None, None, None))
        filters = client.calls[0]["json"]["filterGroups"][0]["filters"]
        assert filters[0]["value"] == "2024-05-15T10:00:00Z"

    def test_cap_triggers_keyset_restart(self, deals_stream):
        client = wire(deals_stream, [search_page([rec(1), rec(2)], next_after="10000"), search_page([rec(3)])])
        out = list(deals_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1", "2", "3"]
        restart = client.calls[1]["json"]
        # Restart drops ``after`` and adds the keyset filter on the last id.
        assert "after" not in restart
        assert restart["filterGroups"][0]["filters"][2] == {
            "propertyName": "hs_object_id",
            "operator": "GT",
            "value": "2",
        }

    def test_cap_restart_without_last_id_stops(self, deals_stream, caplog):
        # Last record on the capped page has no id — keyset filter would be
        # invalid, so the stream stops instead of looping.
        wire(deals_stream, [search_page([{"updatedAt": "2024-06-01T00:00:00Z"}], next_after="10000")])
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            out = list(deals_stream._generate_records(None, None, None))
        assert len(out) == 1
        assert "no id" in caplog.text


class TestReadRecordsPipeline:
    def test_envelope_and_association_enrichment(self, deals_stream):
        assoc_payload = {
            "status": "COMPLETE",
            "results": [{"from": {"id": "1"}, "to": [{"toObjectId": 900}, {"toObjectId": 901}]}],
        }
        client = wire(
            deals_stream,
            [
                search_page(
                    [
                        rec(1, updated="2024-06-02T00:00:00Z", amount="10", my_custom="x"),
                        rec(2, updated="2024-06-01T00:00:00Z", amount="20"),
                    ]
                ),
                FakeResponse(assoc_payload),  # companies batch
                FakeResponse({"results": []}),  # contacts batch
            ],
        )
        out = list(deals_stream.read_records(sync_mode=None))
        assert len(out) == 2
        first = out[0]
        assert first["tenant_id"] == TENANT
        assert first["source_id"] == SOURCE
        assert first["unique_key"] == f"{TENANT}-{SOURCE}-1"
        assert first["data_source"] == "hubspot"
        assert first["properties_amount"] == "10"
        # custom property routed into the JSON blob, not a flat column
        assert "properties_my_custom" not in first
        assert first["custom_fields"] == '{"my_custom":"x"}'
        assert first["associations_companies"] == ["900", "901"]
        assert first["associations_contacts"] == []
        # record 2 absent from the association response keeps empty arrays
        assert out[1]["associations_companies"] == []
        # cursor advanced to the max updatedAt seen
        assert deals_stream.state == {"updatedAt": "2024-06-02T00:00:00Z"}
        # 1 search + 2 association batches (companies, contacts)
        assert len(client.calls) == 3
        assert client.calls[1]["url"].endswith("/crm/v4/associations/deals/companies/batch/read")
        assert client.calls[2]["url"].endswith("/crm/v4/associations/deals/contacts/batch/read")

    def test_no_association_calls_without_targets(self, companies_stream):
        client = wire(companies_stream, [search_page([rec(1)])])
        out = list(companies_stream.read_records(sync_mode=None))
        assert len(out) == 1
        assert len(client.calls) == 1  # search only
        assert not any(k.startswith("associations_") for k in out[0])
