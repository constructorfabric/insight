"""Tests for IncrementalRestSalesforceStream: cursor wiring, slices, SOQL windows."""

from __future__ import annotations

import pendulum
from airbyte_cdk.models import SyncMode
from tests.conftest import make_incremental


class FakeCursor:
    """Duck-typed ConcurrentCursor: only stream_slices() is consumed."""

    def __init__(self, slices):
        self._slices = slices

    def stream_slices(self):
        yield from self._slices


class TestCursorWiring:
    def test_cursor_field_is_replication_key(self, incremental_stream):
        assert incremental_stream.cursor_field == "SystemModstamp"

    def test_state_roundtrip(self, incremental_stream):
        assert incremental_stream.state == {}
        incremental_stream.state = {"SystemModstamp": "2024-05-01T00:00:00Z"}
        assert incremental_stream.state == {"SystemModstamp": "2024-05-01T00:00:00Z"}

    def test_stream_slice_step_parses_iso_duration(self):
        stream = make_incremental(stream_slice_step="P7D")
        assert stream.stream_slice_step == pendulum.duration(days=7)

    def test_set_cursor(self, incremental_stream):
        cursor = FakeCursor([])
        incremental_stream.set_cursor(cursor)
        assert incremental_stream._stream_slicer_cursor is cursor


class TestStreamSlices:
    def test_no_cursor_yields_single_empty_slice(self, incremental_stream):
        slices = list(incremental_stream.stream_slices(sync_mode=SyncMode.incremental))
        assert len(slices) == 1
        assert dict(slices[0]) == {}

    def test_cursor_slices_converted_to_offset_format(self, incremental_stream):
        incremental_stream.set_cursor(
            FakeCursor(
                [
                    {"start_date": "2024-01-01T00:00:00.000Z", "end_date": "2024-01-31T00:00:00.000Z"},
                    {"start_date": "2024-01-31T00:00:00.000Z", "end_date": "2024-02-29T00:00:00.000Z"},
                ]
            )
        )
        slices = list(incremental_stream.stream_slices(sync_mode=SyncMode.incremental))
        assert len(slices) == 2
        assert slices[0]["start_date"] == "2024-01-01T00:00:00.000+00:00"
        assert slices[0]["end_date"] == "2024-01-31T00:00:00.000+00:00"


class TestIncrementalRequestParams:
    CHUNK = {"Id": {}, "SystemModstamp": {}}

    def test_next_page_token_suppresses_params(self, incremental_stream):
        assert incremental_stream.request_params(stream_state={}, next_page_token={"next_token": "/q"}) == {}

    def test_no_cursor_plain_select(self, incremental_stream):
        params = incremental_stream.request_params(stream_state={}, property_chunk=self.CHUNK)
        assert params == {"q": "SELECT Id,SystemModstamp FROM Account"}

    def test_slice_boundaries_build_where_clause(self, incremental_stream):
        incremental_stream.set_cursor(FakeCursor([]))
        params = incremental_stream.request_params(
            stream_state={},
            stream_slice={"start_date": "2024-01-01T00:00:00.000+00:00", "end_date": "2024-01-31T00:00:00.000+00:00"},
            property_chunk=self.CHUNK,
        )
        assert params["q"] == (
            "SELECT Id,SystemModstamp FROM Account "
            "WHERE SystemModstamp >= 2024-01-01T00:00:00.000+00:00 "
            "AND SystemModstamp < 2024-01-31T00:00:00.000+00:00"
        )

    def test_state_wins_over_older_slice_start(self, incremental_stream):
        """The max() of state cursor vs slice start becomes the lower bound."""
        incremental_stream.set_cursor(FakeCursor([]))
        params = incremental_stream.request_params(
            stream_state={"SystemModstamp": "2024-01-15T00:00:00Z"},
            stream_slice={"start_date": "2024-01-01T00:00:00.000+00:00", "end_date": "2024-01-31T00:00:00.000+00:00"},
            property_chunk=self.CHUNK,
        )
        assert ">= 2024-01-15T00:00:00.000+00:00" in params["q"]

    def test_datetime_variants_normalized_to_soql_literals(self, incremental_stream):
        """Non-UTC offsets and second-precision inputs become canonical UTC millis."""
        incremental_stream.set_cursor(FakeCursor([]))
        params = incremental_stream.request_params(
            stream_state={},
            stream_slice={"start_date": "2024-06-01T12:00:00+02:00", "end_date": "2024-06-02T00:00:00Z"},
            property_chunk=self.CHUNK,
        )
        assert ">= 2024-06-01T10:00:00.000+00:00" in params["q"]
        assert "< 2024-06-02T00:00:00.000+00:00" in params["q"]

    def test_unparsable_start_omits_lower_bound(self, incremental_stream):
        incremental_stream.set_cursor(FakeCursor([]))
        stream = incremental_stream
        stream.start_date = None
        params = stream.request_params(
            stream_state={"SystemModstamp": "not-a-date"},
            stream_slice={"end_date": "2024-01-31T00:00:00Z"},
            property_chunk=self.CHUNK,
        )
        assert ">=" not in params["q"]
        assert "WHERE SystemModstamp < 2024-01-31T00:00:00.000+00:00" in params["q"]

    def test_end_date_defaults_to_now(self, incremental_stream):
        incremental_stream.set_cursor(FakeCursor([]))
        incremental_stream.start_date = None
        params = incremental_stream.request_params(stream_state={}, property_chunk=self.CHUNK)
        # Only the upper bound is present, pinned to "now".
        assert ">=" not in params["q"]
        assert "WHERE SystemModstamp < " in params["q"]
        year = pendulum.now(tz="UTC").year
        assert f"< {year}" in params["q"]

    def test_start_date_from_config_used_when_no_state(self):
        stream = make_incremental(start_date="2024-03-01")
        stream.set_cursor(FakeCursor([]))
        params = stream.request_params(stream_state={}, property_chunk=self.CHUNK)
        assert ">= 2024-03-01T00:00:00.000+00:00" in params["q"]

    def test_next_page_token_start_date_considered(self, incremental_stream):
        incremental_stream.set_cursor(FakeCursor([]))
        incremental_stream.start_date = None
        params = incremental_stream.request_params(
            stream_state={},
            next_page_token=None,
            stream_slice={"start_date": "2024-01-01T00:00:00Z", "end_date": "2024-02-01T00:00:00Z"},
            property_chunk=self.CHUNK,
        )
        assert ">= 2024-01-01T00:00:00.000+00:00" in params["q"]
