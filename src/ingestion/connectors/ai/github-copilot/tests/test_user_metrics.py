"""copilot_user_metrics — incremental per-user daily metrics.

The named risk here is the "Major #5" fix: a day that returns HTTP 204 (no
data yet) must still advance the cursor, or every run re-walks the same empty
historical days. The lookback-window re-fetch (self-healing late/restated
reports) is the second risk axis.
"""

from __future__ import annotations

import re

import pytest

from source_github_copilot.streams.user_metrics import CopilotUserMetricsStream
from tests.conftest import FakeResponse, SHARED_STREAM_KWARGS


def _user_metrics(**overrides) -> CopilotUserMetricsStream:
    kwargs = {"start_date": "2026-01-01", "lookback_days": 7, **SHARED_STREAM_KWARGS}
    kwargs.update(overrides)
    return CopilotUserMetricsStream(**kwargs)


class TestInit:
    def test_default_start_date_is_a_valid_iso_date(self):
        stream = CopilotUserMetricsStream(**SHARED_STREAM_KWARGS)
        assert re.match(r"^\d{4}-\d{2}-\d{2}$", stream._start_date)

    def test_custom_start_date_used_verbatim(self):
        stream = _user_metrics(start_date="2026-03-15")
        assert stream._start_date == "2026-03-15"

    def test_lookback_days_negative_clamped_to_zero(self):
        stream = _user_metrics(lookback_days=-5)
        assert stream._lookback_days == 0

    def test_lookback_days_string_coerced_to_int(self):
        stream = _user_metrics(lookback_days="14")
        assert stream._lookback_days == 14

    def test_initial_state_is_empty(self):
        assert _user_metrics().state == {}


class TestStateProperty:
    def test_setter_getter_roundtrip(self):
        stream = _user_metrics()
        stream.state = {"day": "2026-01-05"}
        assert stream.state == {"day": "2026-01-05"}

    def test_none_normalized_to_empty_dict(self):
        stream = _user_metrics()
        stream.state = None
        assert stream.state == {}


class TestPathAndRequestParams:
    def test_path_includes_org(self):
        assert _user_metrics().path() == "orgs/acme/copilot/metrics/reports/users-1-day"

    def test_day_required_in_slice(self):
        with pytest.raises(ValueError):
            _user_metrics().request_params(stream_slice=None)

    def test_day_extracted_from_slice(self):
        params = _user_metrics().request_params(stream_slice={"day": "2026-01-05"})
        assert params == {"day": "2026-01-05"}


class TestStreamSlices:
    def test_first_run_spans_start_date_to_yesterday(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.user_metrics.yesterday_utc", lambda: "2026-01-03"
        )
        stream = _user_metrics(start_date="2026-01-01")
        days = [s["day"] for s in stream.stream_slices()]
        assert days == ["2026-01-01", "2026-01-02", "2026-01-03"]

    def test_resumed_run_uses_trailing_lookback_window(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.user_metrics.yesterday_utc", lambda: "2026-01-12"
        )
        stream = _user_metrics(start_date="2026-01-01", lookback_days=3)
        stream.state = {"day": "2026-01-10"}
        days = [s["day"] for s in stream.stream_slices()]
        # lookback = 01-10 minus 3 days = 01-07, through yesterday (01-12).
        assert days[0] == "2026-01-07"
        assert days[-1] == "2026-01-12"
        assert len(days) == 6

    def test_lookback_window_clamped_at_start_date(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.user_metrics.yesterday_utc", lambda: "2026-01-06"
        )
        stream = _user_metrics(start_date="2026-01-05", lookback_days=30)
        stream.state = {"day": "2026-01-06"}
        days = [s["day"] for s in stream.stream_slices()]
        # lookback would reach back to December, but must not precede start_date.
        assert days[0] == "2026-01-05"

    def test_stream_state_argument_used_when_no_in_memory_state(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.user_metrics.yesterday_utc", lambda: "2026-01-08"
        )
        stream = _user_metrics(start_date="2026-01-01", lookback_days=0)
        days = [s["day"] for s in stream.stream_slices(stream_state={"day": "2026-01-06"})]
        assert days[0] == "2026-01-06"

    def test_invalid_date_yields_no_slices_not_raises(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.user_metrics.yesterday_utc", lambda: "2026-01-08"
        )
        stream = _user_metrics(start_date="not-a-date")
        assert list(stream.stream_slices()) == []


class TestRecordPkPartsAndFilter:
    def test_pk_parts_are_login_then_day(self):
        stream = _user_metrics()
        assert stream._record_pk_parts({"user_login": "alice"}, "2026-01-05") == [
            "alice",
            "2026-01-05",
        ]

    def test_pk_parts_missing_login_is_empty_string(self):
        stream = _user_metrics()
        assert stream._record_pk_parts({}, "2026-01-05") == ["", "2026-01-05"]

    def test_filter_accepts_record_with_login(self):
        assert _user_metrics()._filter_record({"user_login": "alice"}) is True

    def test_filter_rejects_record_without_login(self):
        assert _user_metrics()._filter_record({}) is False


class TestParseResponseDayInjection:
    def test_day_injected_when_missing_from_ndjson(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=200, lines=['{"user_login": "alice"}']),
        )
        envelope = FakeResponse(status_code=200, payload={"download_links": ["https://signed/x"]})
        records = list(
            _user_metrics().parse_response(envelope, stream_slice={"day": "2026-02-01"})
        )
        assert records[0]["day"] == "2026-02-01"

    def test_day_preserved_when_already_present(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(
                status_code=200, lines=['{"user_login": "alice", "day": "2026-02-01"}']
            ),
        )
        envelope = FakeResponse(status_code=200, payload={"download_links": ["https://signed/x"]})
        records = list(
            _user_metrics().parse_response(envelope, stream_slice={"day": "2026-02-01"})
        )
        assert records[0]["day"] == "2026-02-01"

    def test_login_less_record_is_still_dropped_by_filter_before_day_injection(self, monkeypatch):
        """_filter_record runs inside _fetch_ndjson_records before parse_response's
        day-injection ever sees the row — a user_login-less row never surfaces."""
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=200, lines=['{"loc_added_sum": 5}']),
        )
        envelope = FakeResponse(status_code=200, payload={"download_links": ["https://signed/x"]})
        records = list(
            _user_metrics().parse_response(envelope, stream_slice={"day": "2026-02-01"})
        )
        assert records == []


class TestReadRecordsCursorAdvancement:
    """The Major #5 fix: cursor must advance on the slice's day even when the
    upstream fetch yields zero records (HTTP 204 — no data for that day)."""

    def test_204_day_advances_cursor_with_zero_records(self, monkeypatch):
        import airbyte_cdk.sources.streams.http as http_module

        monkeypatch.setattr(http_module.HttpStream, "read_records", lambda self, **kw: iter([]))
        stream = _user_metrics()
        records = list(stream.read_records(sync_mode=None, stream_slice={"day": "2026-02-01"}))
        assert records == []
        assert stream.state == {"day": "2026-02-01"}

    def test_records_are_still_yielded_when_present(self, monkeypatch):
        import airbyte_cdk.sources.streams.http as http_module

        monkeypatch.setattr(
            http_module.HttpStream, "read_records", lambda self, **kw: iter([{"user_login": "a"}])
        )
        stream = _user_metrics()
        records = list(stream.read_records(sync_mode=None, stream_slice={"day": "2026-02-01"}))
        assert records == [{"user_login": "a"}]
        assert stream.state == {"day": "2026-02-01"}

    def test_state_never_regresses_for_an_older_lookback_slice(self, monkeypatch):
        import airbyte_cdk.sources.streams.http as http_module

        monkeypatch.setattr(http_module.HttpStream, "read_records", lambda self, **kw: iter([]))
        stream = _user_metrics()
        stream.state = {"day": "2026-02-05"}
        list(stream.read_records(sync_mode=None, stream_slice={"day": "2026-02-01"}))
        assert stream.state == {"day": "2026-02-05"}

    def test_no_slice_leaves_state_untouched(self, monkeypatch):
        import airbyte_cdk.sources.streams.http as http_module

        monkeypatch.setattr(http_module.HttpStream, "read_records", lambda self, **kw: iter([]))
        stream = _user_metrics()
        list(stream.read_records(sync_mode=None, stream_slice=None))
        assert stream.state == {}


class TestGetJsonSchema:
    def test_grain_and_feature_flags_present(self):
        schema = _user_metrics().get_json_schema()
        props = schema["properties"]
        for field in ("day", "user_login", "used_chat", "used_agent", "used_cli"):
            assert field in props
        assert schema["additionalProperties"] is True
