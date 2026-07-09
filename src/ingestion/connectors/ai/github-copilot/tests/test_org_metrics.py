"""copilot_org_metrics — incremental org-level daily aggregates.

Same two-step signed-URL + IncrementalMixin pattern as user_metrics, but with
no per-record identity dimension: unique_key is keyed on `day` alone (plus the
tenant/source prefix), and there is no _filter_record override — org rows have
no user_login to be missing.
"""

from __future__ import annotations

import re

import pytest

from source_github_copilot.streams.org_metrics import CopilotOrgMetricsStream
from tests.conftest import FakeResponse, SHARED_STREAM_KWARGS


def _org_metrics(**overrides) -> CopilotOrgMetricsStream:
    kwargs = {"start_date": "2026-01-01", "lookback_days": 7, **SHARED_STREAM_KWARGS}
    kwargs.update(overrides)
    return CopilotOrgMetricsStream(**kwargs)


class TestInit:
    def test_default_start_date_is_a_valid_iso_date(self):
        stream = CopilotOrgMetricsStream(**SHARED_STREAM_KWARGS)
        assert re.match(r"^\d{4}-\d{2}-\d{2}$", stream._start_date)

    def test_lookback_days_negative_clamped_to_zero(self):
        assert _org_metrics(lookback_days=-1)._lookback_days == 0

    def test_initial_state_is_empty(self):
        assert _org_metrics().state == {}


class TestPathAndRequestParams:
    def test_path_includes_org(self):
        assert _org_metrics().path() == "orgs/acme/copilot/metrics/reports/organization-1-day"

    def test_day_required_in_slice(self):
        with pytest.raises(ValueError):
            _org_metrics().request_params(stream_slice=None)

    def test_day_extracted_from_slice(self):
        assert _org_metrics().request_params(stream_slice={"day": "2026-01-05"}) == {
            "day": "2026-01-05"
        }


class TestStreamSlices:
    def test_first_run_spans_start_date_to_yesterday(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.org_metrics.yesterday_utc", lambda: "2026-01-03"
        )
        days = [s["day"] for s in _org_metrics(start_date="2026-01-01").stream_slices()]
        assert days == ["2026-01-01", "2026-01-02", "2026-01-03"]

    def test_resumed_run_uses_trailing_lookback_window(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.org_metrics.yesterday_utc", lambda: "2026-01-12"
        )
        stream = _org_metrics(start_date="2026-01-01", lookback_days=3)
        stream.state = {"day": "2026-01-10"}
        days = [s["day"] for s in stream.stream_slices()]
        assert (days[0], days[-1], len(days)) == ("2026-01-07", "2026-01-12", 6)

    def test_lookback_window_clamped_at_start_date(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.org_metrics.yesterday_utc", lambda: "2026-01-06"
        )
        stream = _org_metrics(start_date="2026-01-05", lookback_days=30)
        stream.state = {"day": "2026-01-06"}
        assert list(stream.stream_slices())[0]["day"] == "2026-01-05"

    def test_invalid_date_yields_no_slices_not_raises(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.org_metrics.yesterday_utc", lambda: "2026-01-08"
        )
        assert list(_org_metrics(start_date="not-a-date").stream_slices()) == []


class TestRecordPkParts:
    def test_pk_parts_are_day_only_no_user_dimension(self):
        assert _org_metrics()._record_pk_parts({"daily_active_users": 5}, "2026-01-05") == [
            "2026-01-05"
        ]

    def test_default_filter_accepts_every_record(self):
        """Org rows have no user_login to filter on — unlike user_metrics, every
        NDJSON row must pass through."""
        assert _org_metrics()._filter_record({}) is True


class TestParseResponseDayInjection:
    def test_day_injected_when_missing_from_ndjson(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=200, lines=['{"daily_active_users": 5}']),
        )
        envelope = FakeResponse(status_code=200, payload={"download_links": ["https://signed/x"]})
        records = list(_org_metrics().parse_response(envelope, stream_slice={"day": "2026-02-01"}))
        assert records[0]["day"] == "2026-02-01"
        assert records[0]["unique_key"] == "T-S-2026-02-01"


class TestReadRecordsCursorAdvancement:
    def test_204_day_advances_cursor_with_zero_records(self, monkeypatch):
        import airbyte_cdk.sources.streams.http as http_module

        monkeypatch.setattr(http_module.HttpStream, "read_records", lambda self, **kw: iter([]))
        stream = _org_metrics()
        records = list(stream.read_records(sync_mode=None, stream_slice={"day": "2026-02-01"}))
        assert records == []
        assert stream.state == {"day": "2026-02-01"}

    def test_state_never_regresses_for_an_older_lookback_slice(self, monkeypatch):
        import airbyte_cdk.sources.streams.http as http_module

        monkeypatch.setattr(http_module.HttpStream, "read_records", lambda self, **kw: iter([]))
        stream = _org_metrics()
        stream.state = {"day": "2026-02-05"}
        list(stream.read_records(sync_mode=None, stream_slice={"day": "2026-02-01"}))
        assert stream.state == {"day": "2026-02-05"}

    def test_records_are_still_yielded_when_present(self, monkeypatch):
        import airbyte_cdk.sources.streams.http as http_module

        monkeypatch.setattr(
            http_module.HttpStream,
            "read_records",
            lambda self, **kw: iter([{"daily_active_users": 5}]),
        )
        stream = _org_metrics()
        records = list(stream.read_records(sync_mode=None, stream_slice={"day": "2026-02-01"}))
        assert records == [{"daily_active_users": 5}]
        assert stream.state == {"day": "2026-02-01"}


class TestGetJsonSchema:
    def test_org_aggregate_fields_present_no_user_login(self):
        schema = _org_metrics().get_json_schema()
        props = schema["properties"]
        for field in ("day", "daily_active_users", "pull_requests"):
            assert field in props
        assert "user_login" not in props
        assert schema["additionalProperties"] is True
