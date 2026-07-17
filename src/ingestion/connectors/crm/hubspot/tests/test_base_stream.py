"""HubspotStream base behavior: identity, state management, module helpers."""

from __future__ import annotations

import logging

import pendulum
import pytest
from source_hubspot import streams as streams_mod
from source_hubspot.streams import _after_exceeds_cap, _parse_datetime, _record_cursor_passes
from tests.conftest import START


class TestIdentity:
    def test_name_and_primary_key(self, deals_stream):
        assert deals_stream.name == "deals"
        assert deals_stream.primary_key == "id"

    def test_cursor_field_from_registry(self, deals_stream, companies_archived_stream):
        assert deals_stream.cursor_field == "updatedAt"
        assert companies_archived_stream.cursor_field == "archivedAt"

    def test_auth_header_installed(self, deals_stream):
        # Constructed before wire() swaps the client in tests — the real
        # session must carry the Private App bearer token.
        session = deals_stream._http_client._session
        assert session.headers["Authorization"] == "Bearer pat-test-token"


class TestState:
    def test_empty_until_set(self, deals_stream):
        assert deals_stream.state == {}

    def test_roundtrip_iso_datetime(self, deals_stream):
        deals_stream.state = {"updatedAt": "2024-06-01T12:00:00Z"}
        assert deals_stream._state == pendulum.datetime(2024, 6, 1, 12, tz="UTC")
        assert deals_stream.state == {"updatedAt": "2024-06-01T12:00:00Z"}

    def test_ignores_empty_mapping(self, deals_stream):
        deals_stream.state = {}
        assert deals_stream._state is None

    def test_ignores_missing_cursor_key(self, deals_stream):
        deals_stream.state = {"other": "2024-06-01T00:00:00Z"}
        assert deals_stream._state is None

    def test_ignores_unparseable_value(self, deals_stream):
        deals_stream.state = {"updatedAt": "not-a-date"}
        assert deals_stream._state is None

    def test_date_only_parse_normalized_to_utc_midnight(self, deals_stream, monkeypatch):
        # pendulum.parse returns DateTime for date strings by default; force
        # the Date branch to verify the UTC-midnight normalization.
        monkeypatch.setattr(streams_mod.pendulum, "parse", lambda raw: pendulum.date(2024, 3, 5))
        deals_stream.state = {"updatedAt": "2024-03-05"}
        assert deals_stream._state == pendulum.datetime(2024, 3, 5, tz="UTC")

    def test_advance_state_moves_forward_only(self, deals_stream):
        t1 = pendulum.datetime(2024, 5, 1, tz="UTC")
        t2 = pendulum.datetime(2024, 6, 1, tz="UTC")
        deals_stream._advance_state(None)
        assert deals_stream._state is None
        deals_stream._advance_state(t2)
        assert deals_stream._state == t2
        deals_stream._advance_state(t1)  # older — must not regress
        assert deals_stream._state == t2


class TestRecordCursor:
    def test_extracts_iso_string(self, deals_stream):
        got = deals_stream._record_cursor({"updatedAt": "2024-06-01T00:00:00Z"})
        assert got == pendulum.datetime(2024, 6, 1, tz="UTC")

    def test_passes_through_datetime(self, deals_stream):
        dt = pendulum.datetime(2024, 6, 1, tz="UTC")
        assert deals_stream._record_cursor({"updatedAt": dt}) is dt

    def test_none_when_absent_or_invalid(self, deals_stream):
        assert deals_stream._record_cursor({}) is None
        assert deals_stream._record_cursor({"updatedAt": "garbage"}) is None

    def test_none_when_no_cursor_field(self, deals_stream):
        deals_stream._registry = {"cursor_field": None}
        assert deals_stream._record_cursor({"updatedAt": "2024-06-01T00:00:00Z"}) is None
        assert deals_stream.state == {}

    def test_none_when_parse_yields_date(self, deals_stream, monkeypatch):
        monkeypatch.setattr(streams_mod.pendulum, "parse", lambda raw: pendulum.date(2024, 3, 5))
        assert deals_stream._record_cursor({"updatedAt": "2024-03-05"}) is None


class TestSchema:
    def test_schema_has_envelope_and_association_columns(self, deals_stream):
        schema = deals_stream.get_json_schema()
        props = schema["properties"]
        assert props["properties_amount"] == {"type": ["string", "null"]}
        for field in ("tenant_id", "source_id", "unique_key", "data_source", "collected_at", "custom_fields"):
            assert field in props
        # deals registry declares [companies, contacts]
        assert props["associations_companies"]["type"] == ["array", "null"]
        assert props["associations_contacts"]["items"] == {"type": "string"}

    def test_schema_does_not_mutate_describe_cache(self, deals_stream):
        cached = deals_stream._hubspot.generate_schema("deals")
        before = dict(cached["properties"])
        deals_stream.get_json_schema()
        assert cached["properties"] == before  # deepcopy protects the cache

    def test_no_association_columns_without_associations(self, companies_stream):
        props = companies_stream.get_json_schema()["properties"]
        assert not any(k.startswith("associations_") for k in props)


class TestHelpers:
    @pytest.mark.parametrize(
        "after,expected", [("10000", True), (10001, True), ("9999", False), ("abc", False), (None, False)]
    )
    def test_after_exceeds_cap(self, after, expected):
        assert _after_exceeds_cap(after) is expected

    def test_parse_datetime_branches(self):
        dt = pendulum.datetime(2024, 1, 1, tz="UTC")
        assert _parse_datetime(None) is None
        assert _parse_datetime(dt) is dt
        assert _parse_datetime("2024-01-01T00:00:00Z") == dt
        assert _parse_datetime("garbage") is None

    def test_parse_datetime_rejects_non_datetime_parse(self, monkeypatch):
        monkeypatch.setattr(streams_mod.pendulum, "parse", lambda raw: pendulum.date(2024, 1, 1))
        assert _parse_datetime("2024-01-01") is None

    def test_record_cursor_passes(self, caplog):
        threshold = pendulum.datetime(2024, 1, 1, tz="UTC")
        rec_at = {"archivedAt": "2024-01-01T00:00:00Z", "id": "1"}
        rec_after = {"archivedAt": "2024-02-01T00:00:00Z", "id": "2"}
        # exclusive (default): boundary record dropped
        assert _record_cursor_passes(rec_at, "archivedAt", threshold) is False
        assert _record_cursor_passes(rec_after, "archivedAt", threshold) is True
        # inclusive: boundary record kept
        assert _record_cursor_passes(rec_at, "archivedAt", threshold, inclusive=True) is True
        # no threshold: everything with a parseable cursor passes
        assert _record_cursor_passes(rec_at, "archivedAt", None) is True
        # missing/unparseable cursor: dropped with a warning
        with caplog.at_level(logging.WARNING, logger="airbyte"):
            assert _record_cursor_passes({"id": "3"}, "archivedAt", None) is False
        assert "no parseable" in caplog.text


class TestReadRecordsBatching:
    def test_flushes_in_page_limit_batches(self, companies_stream, monkeypatch):
        # Shrink the flush threshold so three records force two batches.
        monkeypatch.setattr(streams_mod, "SEARCH_PAGE_LIMIT", 2)
        flushed: list[int] = []
        original = companies_stream._finalize_batch

        def spy(batch, custom_names):
            flushed.append(len(batch))
            return original(batch, custom_names)

        companies_stream._finalize_batch = spy
        records = [{"id": str(i), "updatedAt": f"2024-06-0{i}T00:00:00Z", "properties": {}} for i in (1, 2, 3)]
        monkeypatch.setattr(companies_stream, "_generate_records", lambda *a, **kw: iter(records))
        out = list(companies_stream.read_records(sync_mode=None))
        assert [r["id"] for r in out] == ["1", "2", "3"]
        assert flushed == [2, 1]
        # State advanced to the max cursor seen.
        assert companies_stream.state == {"updatedAt": "2024-06-03T00:00:00Z"}

    def test_incoming_stream_state_mirrored(self, companies_stream, monkeypatch):
        monkeypatch.setattr(companies_stream, "_generate_records", lambda *a, **kw: iter(()))
        list(companies_stream.read_records(sync_mode=None, stream_state={"updatedAt": "2024-05-01T00:00:00Z"}))
        assert companies_stream._state == pendulum.datetime(2024, 5, 1, tz="UTC")

    def test_start_date_default(self, companies_stream):
        assert companies_stream._start_date == START
