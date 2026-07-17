"""CrmArchivedListStream: two-pass list + batch_read with archivedAt overlay."""

from __future__ import annotations

from source_hubspot import streams as streams_mod
from source_hubspot.constants import BASE_URL
from tests.conftest import SOURCE, TENANT, FakeResponse, wire


def list_page(records, next_after=None):
    payload = {"results": records}
    if next_after is not None:
        payload["paging"] = {"next": {"after": next_after}}
    return FakeResponse(payload)


def stub(i, archived_at="2024-06-01T00:00:00Z"):
    """Pass-1 list stub: id + timestamps, no property values."""
    return {"id": str(i), "archivedAt": archived_at, "archived": True}


def batch_result(records):
    return FakeResponse({"results": records})


class TestPassOneList:
    def test_list_params_have_no_properties(self, companies_archived_stream):
        client = wire(companies_archived_stream, [list_page([])])
        list(companies_archived_stream._generate_records(None, None, None))
        call = client.calls[0]
        assert call["method"] == "GET"
        assert call["url"] == f"{BASE_URL}/crm/v3/objects/companies"
        assert call["params"] == {"limit": 100, "archived": "true"}
        assert "properties" not in call["params"]

    def test_pagination_follows_after(self, companies_archived_stream):
        client = wire(companies_archived_stream, [list_page([], next_after="A1"), list_page([])])
        list(companies_archived_stream._generate_records(None, None, None))
        assert client.calls[1]["params"]["after"] == "A1"

    def test_threshold_filter_applied_before_batch_read(self, companies_archived_stream):
        # Only the record archived after start_date survives Pass 1, so
        # Pass 2 must be called with exactly one id.
        client = wire(
            companies_archived_stream,
            [
                list_page(
                    [
                        stub(1, archived_at="2023-06-01T00:00:00Z"),  # before start
                        stub(2, archived_at="2024-06-01T00:00:00Z"),
                    ]
                ),
                batch_result([{"id": "2", "properties": {}}]),
            ],
        )
        out = list(companies_archived_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["2"]
        assert client.calls[1]["json"]["inputs"] == [{"id": "2"}]

    def test_stub_without_id_skipped(self, companies_archived_stream):
        wire(companies_archived_stream, [list_page([{"archivedAt": "2024-06-01T00:00:00Z"}])])
        # No surviving ids → no batch_read call, no records.
        assert list(companies_archived_stream._generate_records(None, None, None)) == []

    def test_later_sync_excludes_state_boundary(self, companies_archived_stream):
        companies_archived_stream.state = {"archivedAt": "2024-06-01T00:00:00Z"}
        wire(companies_archived_stream, [list_page([stub(1, archived_at="2024-06-01T00:00:00Z")])])
        assert list(companies_archived_stream._generate_records(None, None, None)) == []


class TestPassTwoBatchRead:
    def test_batch_read_request_shape(self, companies_archived_stream):
        client = wire(
            companies_archived_stream,
            [list_page([stub(1)]), batch_result([{"id": "1", "properties": {"name": "Acme"}}])],
        )
        out = list(companies_archived_stream._generate_records(None, None, None))
        assert len(out) == 1
        call = client.calls[1]
        assert call["method"] == "POST"
        # archived=true rides on the URL so batch_read resolves archived ids.
        assert call["url"] == (f"{BASE_URL}/crm/v3/objects/companies/batch/read?archived=true")
        # Full property list (standard + custom) goes in the body.
        assert call["json"]["properties"] == ["amount", "my_custom"]

    def test_pass_one_archived_at_overlays_batch_value(self, companies_archived_stream):
        wire(
            companies_archived_stream,
            [
                list_page([stub(1, archived_at="2024-06-05T00:00:00Z"), stub(2, archived_at="2024-06-06T00:00:00Z")]),
                batch_result(
                    [
                        {"id": "1", "properties": {}},  # archivedAt omitted
                        {"id": "2", "archivedAt": "1970-01-01T00:00:00Z", "properties": {}},  # stale value
                    ]
                ),
            ],
        )
        out = {r["id"]: r for r in companies_archived_stream._generate_records(None, None, None)}
        assert out["1"]["archivedAt"] == "2024-06-05T00:00:00Z"
        assert out["2"]["archivedAt"] == "2024-06-06T00:00:00Z"

    def test_batch_record_without_id_passes_through(self, companies_archived_stream):
        wire(companies_archived_stream, [list_page([stub(1)]), batch_result([{"properties": {"name": "orphan"}}])])
        out = list(companies_archived_stream._generate_records(None, None, None))
        assert out == [{"properties": {"name": "orphan"}}]

    def test_chunking_at_batch_read_limit(self, companies_archived_stream, monkeypatch):
        monkeypatch.setattr(streams_mod, "BATCH_READ_LIMIT", 2)
        client = wire(
            companies_archived_stream,
            [
                list_page([stub(1), stub(2), stub(3)]),
                batch_result([{"id": "1", "properties": {}}, {"id": "2", "properties": {}}]),
                batch_result([{"id": "3", "properties": {}}]),
            ],
        )
        out = list(companies_archived_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1", "2", "3"]
        assert client.calls[1]["json"]["inputs"] == [{"id": "1"}, {"id": "2"}]
        assert client.calls[2]["json"]["inputs"] == [{"id": "3"}]


class TestArchivedReadRecords:
    def test_envelope_and_state_advance(self, companies_archived_stream):
        wire(
            companies_archived_stream,
            [
                list_page([stub(1, archived_at="2024-06-05T00:00:00Z"), stub(2, archived_at="2024-06-07T00:00:00Z")]),
                batch_result(
                    [
                        {"id": "1", "properties": {"amount": "5", "my_custom": "c"}},
                        {"id": "2", "properties": {"amount": "6"}},
                    ]
                ),
            ],
        )
        out = list(companies_archived_stream.read_records(sync_mode=None))
        assert out[0]["unique_key"] == f"{TENANT}-{SOURCE}-1"
        assert out[0]["properties_amount"] == "5"
        assert out[0]["custom_fields"] == '{"my_custom":"c"}'
        # State advanced to max archivedAt (from the Pass-1 overlay).
        assert companies_archived_stream.state == {"archivedAt": "2024-06-07T00:00:00Z"}
