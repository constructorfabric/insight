"""OwnersStream / OwnersArchivedStream: list pagination + client-side filter."""

from __future__ import annotations

from source_hubspot.constants import BASE_URL
from tests.conftest import SOURCE, TENANT, FakeResponse, wire


def owners_page(records, next_after=None):
    payload = {"results": records}
    if next_after is not None:
        payload["paging"] = {"next": {"after": next_after}}
    return FakeResponse(payload)


def owner(i, updated="2024-06-01T00:00:00Z", **extra):
    return {"id": str(i), "email": f"o{i}@x", "updatedAt": updated, **extra}


class TestOwnersSchema:
    def test_hardcoded_schema_with_envelope(self, owners_stream):
        props = owners_stream.get_json_schema()["properties"]
        assert props["email"] == {"type": ["string", "null"]}
        assert props["archivedAt"]["format"] == "date-time"
        assert "unique_key" in props and "custom_fields" in props

    def test_archived_stream_inherits_schema(self, owners_archived_stream):
        assert "userId" in owners_archived_stream.get_json_schema()["properties"]


class TestOwnersPagination:
    def test_single_page_params(self, owners_stream):
        client = wire(owners_stream, [owners_page([owner(1)])])
        out = list(owners_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1"]
        call = client.calls[0]
        assert call["url"] == f"{BASE_URL}/crm/v3/owners/"
        assert call["params"] == {"limit": 100, "archived": "false"}

    def test_follows_after_cursor(self, owners_stream):
        client = wire(owners_stream, [owners_page([owner(1)], next_after="A1"), owners_page([owner(2)])])
        out = list(owners_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1", "2"]
        assert "after" not in client.calls[0]["params"]
        assert client.calls[1]["params"]["after"] == "A1"

    def test_malformed_paging_stops(self, owners_stream):
        wire(owners_stream, [FakeResponse({"results": [owner(1)], "paging": "oops"})])
        assert len(list(owners_stream._generate_records(None, None, None))) == 1


class TestOwnersIncremental:
    def test_first_sync_emits_all(self, owners_stream):
        wire(
            owners_stream,
            [owners_page([owner(1, updated="2020-01-01T00:00:00Z"), owner(2, updated="2024-06-01T00:00:00Z")])],
        )
        out = list(owners_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1", "2"]

    def test_state_filters_unchanged_owners(self, owners_stream):
        owners_stream.state = {"updatedAt": "2024-06-01T00:00:00Z"}
        wire(
            owners_stream,
            [
                owners_page(
                    [
                        owner(1, updated="2024-06-01T00:00:00Z"),  # == state: dropped
                        owner(2, updated="2024-06-02T00:00:00Z"),  # newer: kept
                    ]
                )
            ],
        )
        out = list(owners_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["2"]


class TestOwnersReadRecords:
    def test_envelope_and_state_advance(self, owners_stream):
        wire(
            owners_stream,
            [owners_page([owner(1, updated="2024-06-02T00:00:00Z"), owner(2, updated="2024-06-03T00:00:00Z")])],
        )
        out = list(owners_stream.read_records(sync_mode=None))
        assert out[0]["unique_key"] == f"{TENANT}-{SOURCE}-1"
        assert out[0]["tenant_id"] == TENANT
        assert out[0]["custom_fields"] == "{}"  # owners have no custom fields
        assert owners_stream.state == {"updatedAt": "2024-06-03T00:00:00Z"}

    def test_incoming_stream_state_applied(self, owners_stream):
        wire(
            owners_stream,
            [owners_page([owner(1, updated="2024-06-01T00:00:00Z"), owner(2, updated="2024-07-01T00:00:00Z")])],
        )
        out = list(owners_stream.read_records(sync_mode=None, stream_state={"updatedAt": "2024-06-15T00:00:00Z"}))
        assert [r["id"] for r in out] == ["2"]


class TestOwnersArchived:
    def test_archived_param_and_cursor_field(self, owners_archived_stream):
        client = wire(owners_archived_stream, [owners_page([])])
        list(owners_archived_stream._generate_records(None, None, None))
        assert client.calls[0]["params"]["archived"] == "true"
        assert owners_archived_stream.cursor_field == "archivedAt"

    def test_first_sync_inclusive_at_start_date(self, owners_archived_stream):
        # Record archived exactly at start_date must survive the boundary.
        wire(
            owners_archived_stream,
            [
                owners_page(
                    [
                        owner(1, archivedAt="2024-01-01T00:00:00Z"),
                        owner(2, archivedAt="2023-12-31T23:59:59Z"),  # before start: dropped
                    ]
                )
            ],
        )
        out = list(owners_archived_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["1"]

    def test_later_sync_exclusive_above_state(self, owners_archived_stream):
        owners_archived_stream.state = {"archivedAt": "2024-06-01T00:00:00Z"}
        wire(
            owners_archived_stream,
            [
                owners_page(
                    [
                        owner(1, archivedAt="2024-06-01T00:00:00Z"),  # == state: dropped
                        owner(2, archivedAt="2024-06-02T00:00:00Z"),
                    ]
                )
            ],
        )
        out = list(owners_archived_stream._generate_records(None, None, None))
        assert [r["id"] for r in out] == ["2"]
