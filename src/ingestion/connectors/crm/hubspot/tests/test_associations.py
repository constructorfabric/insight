"""AssociationFetcher: v4 batch_read flattening and record enrichment."""

from __future__ import annotations

import logging

from source_hubspot import associations as assoc_mod
from source_hubspot.associations import AssociationFetcher, _chunked, _parse_association_response
from source_hubspot.constants import BASE_URL
from tests.conftest import FakeHttpClient, FakeResponse


def make_fetcher(responses=(), to_types=("companies",)):
    client = FakeHttpClient(responses)
    fetcher = AssociationFetcher(from_object_type="deals", to_object_types=list(to_types), http_client=client)
    return fetcher, client


def assoc_page(results):
    return FakeResponse({"status": "COMPLETE", "results": results})


class TestEnrich:
    def test_empty_inputs_are_noops(self):
        fetcher, client = make_fetcher()
        assert fetcher.enrich([]) == []
        fetcher_no_targets, _ = make_fetcher(to_types=())
        records = [{"id": "1"}]
        assert fetcher_no_targets.enrich(records) is records
        assert client.calls == []

    def test_records_without_ids_seed_empty_arrays_without_http(self):
        fetcher, client = make_fetcher()
        records = [{"name": "no-id"}]
        out = fetcher.enrich(records)
        assert out[0]["associations_companies"] == []
        assert client.calls == []  # nothing fetchable → no batch call

    def test_associations_inlined_per_target(self):
        fetcher, client = make_fetcher(
            responses=[
                assoc_page([{"from": {"id": "1"}, "to": [{"toObjectId": 10}, {"toObjectId": 11}]}]),
                assoc_page([{"from": {"id": "2"}, "to": [{"toObjectId": 20}]}]),
            ],
            to_types=("companies", "contacts"),
        )
        records = [{"id": "1"}, {"id": 2}]
        fetcher.enrich(records)
        assert records[0]["associations_companies"] == ["10", "11"]
        assert records[0]["associations_contacts"] == []
        assert records[1]["associations_companies"] == []
        assert records[1]["associations_contacts"] == ["20"]
        assert client.calls[0]["url"] == (f"{BASE_URL}/crm/v4/associations/deals/companies/batch/read")
        assert client.calls[1]["url"].endswith("/deals/contacts/batch/read")
        # ids are stringified in the request body
        assert client.calls[0]["json"] == {"inputs": [{"id": "1"}, {"id": "2"}]}

    def test_batches_split_at_chunk_size(self, monkeypatch):
        monkeypatch.setattr(assoc_mod, "ASSOCIATIONS_BATCH_SIZE", 2)
        fetcher, client = make_fetcher(responses=[assoc_page([]), assoc_page([])])
        fetcher.enrich([{"id": "1"}, {"id": "2"}, {"id": "3"}])
        assert len(client.calls) == 2
        assert client.calls[0]["json"]["inputs"] == [{"id": "1"}, {"id": "2"}]
        assert client.calls[1]["json"]["inputs"] == [{"id": "3"}]

    def test_unknown_from_id_in_response_ignored(self):
        fetcher, _ = make_fetcher(responses=[assoc_page([{"from": {"id": "999"}, "to": [{"toObjectId": 1}]}])])
        records = [{"id": "1"}]
        fetcher.enrich(records)
        assert records[0]["associations_companies"] == []


class TestFetchBatch:
    def test_non_json_response_returns_empty(self, caplog):
        fetcher, _ = make_fetcher(responses=[FakeResponse(ValueError("not json"), text="<html>")])
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            assert fetcher._fetch_batch("companies", ["1"]) == {}
        assert "non-JSON" in caplog.text


class TestParseAssociationResponse:
    def test_v4_shape(self):
        payload = {"status": "COMPLETE", "results": [{"from": {"id": "1"}, "to": [{"toObjectId": 456}, {"id": "789"}]}]}
        assert _parse_association_response(payload) == {"1": ["456", "789"]}

    def test_non_mapping_payload(self):
        assert _parse_association_response("nope") == {}
        assert _parse_association_response(None) == {}

    def test_non_mapping_items_skipped(self):
        assert _parse_association_response({"results": ["junk", 42]}) == {}

    def test_missing_from_id_skipped(self):
        payload = {"results": [{"from": {}, "to": [{"toObjectId": 1}]}]}
        assert _parse_association_response(payload) == {}

    def test_to_entries_filtered(self):
        payload = {"results": [{"from": {"id": "1"}, "to": ["junk", {"noise": True}, {"toObjectId": 2}]}]}
        # non-mapping entries skipped; mapping without any id skipped
        assert _parse_association_response(payload) == {"1": ["2"]}

    def test_empty_to_list(self):
        payload = {"results": [{"from": {"id": "1"}}]}
        assert _parse_association_response(payload) == {"1": []}


class TestChunked:
    def test_exact_and_remainder_chunks(self):
        assert list(_chunked(["a", "b", "c"], 2)) == [["a", "b"], ["c"]]
        assert list(_chunked([], 2)) == []
