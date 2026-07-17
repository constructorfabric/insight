"""Tests for source_salesforce.streams: SOQL construction, pagination,
property chunking and record reassembly, envelope injection, substream batching.
"""

from __future__ import annotations

import urllib.parse
from unittest.mock import Mock

import pytest
from airbyte_cdk.models import SyncMode
from airbyte_cdk.sources.streams.http import HttpStream
from requests import exceptions
from source_salesforce.streams import (
    BatchedSubStream,
    PropertyChunk,
    RestSalesforceStream,
    RestSalesforceSubStream,
    SalesforceStream,
)
from tests.conftest import ACCOUNT_SCHEMA, INSTANCE_URL, FakeResponse, make_sf, make_stream


def big_schema(n_fields: int = 4000):
    """Schema wide enough to trip the SOQL length limit (forces chunking)."""
    props = {"Id": {"type": ["string", "null"]}}
    for i in range(n_fields):
        props[f"Field{i:05d}"] = {"type": ["string", "null"]}
    return {"type": "object", "properties": props}


# ---------------------------------------------------------------------------
# SalesforceStream basics
# ---------------------------------------------------------------------------


class TestSalesforceStreamBasics:
    def test_identity_properties(self, stream):
        assert stream.name == "Account"
        # PK is the tenant-scoped unique_key, never the bare SF Id.
        assert stream.primary_key == "unique_key"
        assert stream.url_base == INSTANCE_URL

    def test_format_start_date(self):
        assert SalesforceStream.format_start_date("2021-07-25") == "2021-07-25T00:00:00Z"
        assert SalesforceStream.format_start_date("2021-07-25T10:20:30Z") == "2021-07-25T10:20:30Z"
        assert SalesforceStream.format_start_date(None) is None

    def test_max_properties_length(self, stream):
        assert stream.max_properties_length == 16_384 - len(INSTANCE_URL) - 2000

    def test_too_many_properties(self):
        assert make_stream().too_many_properties is False
        assert make_stream(schema=big_schema()).too_many_properties is True

    def test_parse_response_yields_records(self, stream):
        response = FakeResponse({"records": [{"Id": "1"}, {"Id": "2"}], "done": True})
        assert list(stream.parse_response(response)) == [{"Id": "1"}, {"Id": "2"}]

    def test_connection_error_display_message(self, stream):
        message = stream.get_error_display_message(exceptions.ConnectionError("x"))
        assert "network error" in message
        assert stream.get_error_display_message(ValueError("x")) is None

    def test_sf_properties_lazily_generated(self):
        sf = make_sf()
        sf.generate_schema = Mock(return_value=ACCOUNT_SCHEMA)
        stream = make_stream(sf=sf, schema=None)
        assert stream._sf_properties() == ACCOUNT_SCHEMA["properties"]
        sf.generate_schema.assert_called_once_with("Account")


class TestGetJsonSchema:
    def test_strips_custom_fields_and_injects_envelope(self, stream):
        schema = stream.get_json_schema()
        # Custom __c fields are routed into the custom_fields blob instead.
        assert "Custom__c" not in schema["properties"]
        assert "Id" in schema["properties"]
        for envelope_field in ("tenant_id", "source_id", "unique_key", "data_source", "collected_at", "custom_fields"):
            assert envelope_field in schema["properties"]

    def test_generates_schema_when_missing(self):
        sf = make_sf()
        sf.generate_schema = Mock(return_value=dict(ACCOUNT_SCHEMA))
        stream = make_stream(sf=sf)
        stream.schema = None  # force the lazy describe path
        schema = stream.get_json_schema()
        sf.generate_schema.assert_called_once_with("Account")
        assert "unique_key" in schema["properties"]


class TestReadRecordsEnvelope:
    def test_mappings_enveloped_others_passed_through(self, stream, monkeypatch):
        state_marker = object()  # simulate a non-record message
        monkeypatch.setattr(
            HttpStream,
            "read_records",
            lambda self, *args, **kwargs: iter([{"Id": "001", "Name": "Acme", "Custom__c": "x"}, state_marker]),
        )
        out = list(stream.read_records(SyncMode.full_refresh))
        assert out[0]["unique_key"] == "T-S-001"
        assert out[0]["custom_fields"] == '{"Custom__c":"x"}'
        assert "Custom__c" not in out[0]
        assert out[1] is state_marker


# ---------------------------------------------------------------------------
# RestSalesforceStream: pagination + SOQL
# ---------------------------------------------------------------------------


class TestChunkingGuard:
    def test_too_many_properties_without_pk_raises(self):
        with pytest.raises(RuntimeError, match="no primary key"):
            make_stream(schema=big_schema(), pk=None)

    def test_small_schema_without_pk_is_fine(self):
        assert make_stream(pk=None).pk is None


class TestPath:
    def test_default_path_targets_query_all(self, stream):
        assert stream.path() == f"/services/data/{stream.sf_api.version}/queryAll"

    def test_next_page_token_path(self, stream):
        token = {"next_token": "/services/data/vXX.X/query/01g-2000"}
        assert stream.path(next_page_token=token) == "/services/data/vXX.X/query/01g-2000"


class TestNextPageToken:
    def test_returns_next_records_url(self, stream):
        response = FakeResponse({"nextRecordsUrl": "/q/01g-2000", "records": []})
        assert stream.next_page_token(response) == {"next_token": "/q/01g-2000"}

    def test_none_when_done(self, stream):
        assert stream.next_page_token(FakeResponse({"done": True, "records": []})) is None


class TestRequestParams:
    def test_soql_select_with_order_by(self, stream):
        params = stream.request_params(stream_state={}, property_chunk={"Id": {}, "Name": {}})
        assert params == {"q": "SELECT Id,Name FROM Account  ORDER BY Id ASC"}

    def test_next_page_token_suppresses_params(self, stream):
        assert stream.request_params(stream_state={}, next_page_token={"next_token": "/q"}) == {}

    def test_unsupported_filtering_stream_has_no_order_by(self):
        stream = make_stream(stream_name="TabDefinition")
        params = stream.request_params(stream_state={}, property_chunk={"Id": {}})
        assert "ORDER BY" not in params["q"]

    def test_parent_object_gets_where_in_clause(self):
        stream = make_stream(cls=RestSalesforceStream, stream_name="ContentDocumentLink")
        params = stream.request_params(
            stream_state={}, stream_slice={"parents": [{"Id": "069A"}, {"Id": "069B"}]}, property_chunk={"Id": {}}
        )
        # ContentDocumentLink is both parent-scoped and unsupported-filtering.
        assert params["q"] == ("SELECT Id FROM ContentDocumentLink  WHERE ContentDocumentId IN ('069A','069B')")


class TestChunkProperties:
    def test_single_chunk_for_small_schema(self, stream):
        chunks = list(stream.chunk_properties())
        assert len(chunks) == 1
        assert set(chunks[0]) == set(ACCOUNT_SCHEMA["properties"])

    def test_wide_schema_split_with_pk_in_every_chunk(self):
        stream = make_stream(schema=big_schema())
        chunks = list(stream.chunk_properties())
        assert len(chunks) > 1
        for chunk in chunks:
            assert "Id" in chunk
        # Each chunk stays under the SOQL length budget.
        for chunk in chunks:
            encoded = urllib.parse.quote(",".join(chunk))
            assert len(encoded) < stream.max_properties_length + 100

    def test_no_pk_chunks_have_no_key_prefix(self):
        stream = make_stream(pk=None)
        chunks = list(stream.chunk_properties())
        assert len(chunks) == 1


class TestNextChunkId:
    def test_picks_least_read_non_exhausted(self):
        chunk_a = PropertyChunk({"Id": {}})
        chunk_a.first_time = False
        chunk_a.next_page = {"next_token": "/q"}
        chunk_a.record_counter = 10
        chunk_b = PropertyChunk({"Id": {}})
        chunk_b.first_time = False
        chunk_b.next_page = {"next_token": "/q"}
        chunk_b.record_counter = 3
        assert RestSalesforceStream._next_chunk_id({0: chunk_a, 1: chunk_b}) == 1

    def test_none_when_all_exhausted(self):
        chunk = PropertyChunk({"Id": {}})
        chunk.first_time = False
        chunk.next_page = None
        assert RestSalesforceStream._next_chunk_id({0: chunk}) is None


def _records_generator(request, response, stream_state, stream_slice):
    yield from response.json()["records"]


class TestReadPages:
    def test_single_chunk_pagination(self, stream, monkeypatch):
        """No property chunking: records stream straight through, page by page."""
        pages = [
            FakeResponse({"records": [{"Id": "1"}, {"Id": "2"}], "nextRecordsUrl": "/q/2"}),
            FakeResponse({"records": [{"Id": "3"}]}),
        ]
        fetches = []

        def fake_fetch(stream_slice, stream_state, next_page, properties):
            fetches.append(next_page)
            return None, pages[len(fetches) - 1]

        monkeypatch.setattr(stream, "_fetch_next_page_for_chunk", fake_fetch)
        records = list(stream._read_pages(_records_generator))
        assert records == [{"Id": "1"}, {"Id": "2"}, {"Id": "3"}]
        # Second fetch carried the nextRecordsUrl token.
        assert fetches == [None, {"next_token": "/q/2"}]

    def test_chunked_records_reassembled_by_pk(self, monkeypatch):
        """Wide schema: each chunk returns a partial record; parts merge on Id."""
        stream = make_stream(schema=big_schema())
        n_chunks = len(list(stream.chunk_properties()))
        assert n_chunks > 1
        calls = []

        def fake_fetch(stream_slice, stream_state, next_page, properties):
            calls.append(properties)
            part = {"Id": "001", f"Part{len(calls)}": len(calls)}
            return None, FakeResponse({"records": [part]})

        monkeypatch.setattr(stream, "_fetch_next_page_for_chunk", fake_fetch)
        records = list(stream._read_pages(_records_generator))
        assert len(records) == 1
        merged = records[0]
        assert merged["Id"] == "001"
        for i in range(1, n_chunks + 1):
            assert merged[f"Part{i}"] == i

    def test_inconsistent_records_skipped_with_warning(self, monkeypatch, caplog):
        """A record seen by only one chunk is dropped, not emitted half-empty."""
        stream = make_stream(schema=big_schema())
        calls = []

        def fake_fetch(stream_slice, stream_state, next_page, properties):
            calls.append(properties)
            records = [{"Id": "001"}]
            if len(calls) == 1:  # phantom row visible to the first chunk only
                records.append({"Id": "002"})
            return None, FakeResponse({"records": records})

        monkeypatch.setattr(stream, "_fetch_next_page_for_chunk", fake_fetch)
        with caplog.at_level("WARNING"):
            records = list(stream._read_pages(_records_generator))
        assert [r["Id"] for r in records] == ["001"]
        assert "Inconsistent record(s) with primary keys 002" in caplog.text


class TestFetchNextPageForChunk:
    def test_sends_request_through_http_client(self, stream):
        sent = {}

        def send_request(**kwargs):
            sent.update(kwargs)
            return "req", FakeResponse({"records": []})

        stream._http_client = Mock(send_request=send_request)
        request, response = stream._fetch_next_page_for_chunk(property_chunk={"Id": {}, "Name": {}})
        assert request == "req"
        assert sent["http_method"] == "GET"
        assert sent["url"].startswith(INSTANCE_URL)
        assert sent["params"]["q"].startswith("SELECT Id,Name FROM Account")


# ---------------------------------------------------------------------------
# BatchedSubStream
# ---------------------------------------------------------------------------


class FakeParentStream:
    """Duck-typed parent: HttpSubStream.stream_slices only calls read_only_records."""

    def __init__(self, records):
        self._records = records

    def read_only_records(self, stream_state=None):
        yield from self._records


class TestBatchedSubStream:
    def _substream(self, parent_records, batch_size=2):
        stream = make_stream(
            cls=RestSalesforceSubStream, stream_name="ContentDocumentLink", parent=FakeParentStream(parent_records)
        )
        stream.SLICE_BATCH_SIZE = batch_size
        return stream

    def test_parents_batched_into_slices(self):
        parents = [{"Id": f"069{i}"} for i in range(5)]
        stream = self._substream(parents, batch_size=2)
        slices = list(stream.stream_slices(SyncMode.full_refresh))
        assert [len(s["parents"]) for s in slices] == [2, 2, 1]
        assert slices[0]["parents"][0] == {"Id": "0690"}

    def test_exact_multiple_has_no_empty_tail(self):
        parents = [{"Id": "A"}, {"Id": "B"}]
        slices = list(self._substream(parents, batch_size=2).stream_slices(SyncMode.full_refresh))
        assert len(slices) == 1

    def test_no_parents_yields_nothing(self):
        assert list(self._substream([]).stream_slices(SyncMode.full_refresh)) == []

    def test_default_batch_size(self):
        assert BatchedSubStream.SLICE_BATCH_SIZE == 200
