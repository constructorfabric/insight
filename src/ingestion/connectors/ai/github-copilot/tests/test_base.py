"""Base stream classes — the shared retry/auth/envelope machinery both metrics
streams build on. This is where the named risks live:

  - rate-limit vs auth-failure 403 disambiguation (_is_rate_limit_403)
  - the bounded signed-URL-expiry refetch loop (DESIGN §3.6)
  - malformed-input tolerance at every parse boundary (non-JSON envelope,
    empty download_links, bad NDJSON lines)
"""

from __future__ import annotations

import pytest
import requests

import re

from source_github_copilot.streams.base import (
    CopilotAuthError,
    CopilotReportsStream,
    SignedUrlExpired,
    _is_rate_limit_403,
    _make_unique_key,
    yesterday_utc,
)
from source_github_copilot.streams.seats import CopilotSeatsStream
from tests.conftest import FakeResponse, SHARED_STREAM_KWARGS, real_response


def _seats() -> CopilotSeatsStream:
    return CopilotSeatsStream(**SHARED_STREAM_KWARGS)


class _MinimalReportsStream(CopilotReportsStream):
    """Concrete CopilotReportsStream with no subclass-specific overrides, so
    tests here exercise only the shared base-class envelope/retry machinery —
    not user_metrics'/org_metrics' own day-injection or cursor logic."""

    name = "test_reports_stream"

    def path(self, **kwargs) -> str:
        return "orgs/acme/copilot/metrics/reports/test"

    def request_params(self, stream_slice=None, **kwargs):
        return {"day": (stream_slice or {}).get("day", "")}

    def _record_pk_parts(self, record, day):
        return [str(record.get("id", "")), day]


def _reports() -> _MinimalReportsStream:
    return _MinimalReportsStream(**SHARED_STREAM_KWARGS)


class TestIsRateLimit403:
    def test_non_403_is_never_rate_limit(self):
        assert _is_rate_limit_403(real_response(status_code=500)) is False

    def test_retry_after_header_signals_rate_limit(self):
        resp = real_response(status_code=403, headers={"Retry-After": "30"})
        assert _is_rate_limit_403(resp) is True

    def test_zero_remaining_header_signals_rate_limit(self):
        resp = real_response(status_code=403, headers={"X-RateLimit-Remaining": "0"})
        assert _is_rate_limit_403(resp) is True

    def test_rate_limit_phrase_in_body_signals_rate_limit(self):
        resp = real_response(status_code=403, content=b"You have exceeded a secondary rate limit")
        assert _is_rate_limit_403(resp) is True

    def test_plain_scope_denial_is_not_rate_limit(self):
        """The disambiguation that matters: a bare scope-403 must NOT be retried."""
        resp = real_response(status_code=403, content=b"Resource not accessible by personal access token")
        assert _is_rate_limit_403(resp) is False

    def test_text_access_failure_is_tolerated(self):
        class _BoomText:
            status_code = 403
            headers: dict = {}

            @property
            def text(self):
                raise RuntimeError("boom")

        assert _is_rate_limit_403(_BoomText()) is False


class TestYesterdayUtc:
    def test_returns_iso_date_string(self):
        assert re.match(r"^\d{4}-\d{2}-\d{2}$", yesterday_utc())


class TestMakeUniqueKey:
    def test_hyphen_joins_tenant_source_and_natural_key(self):
        assert _make_unique_key("T", "S", "user-x", "2026-01-01") == "T-S-user-x-2026-01-01"

    def test_no_natural_key_parts(self):
        assert _make_unique_key("T", "S") == "T-S"

    def test_non_string_parts_are_coerced(self):
        assert _make_unique_key("T", "S", 42) == "T-S-42"


class TestRequestHeaders:
    def test_bearer_token_present(self):
        headers = _seats().request_headers()
        assert headers["Authorization"] == "Bearer tok"


class TestRequestTimeout:
    def test_sixty_seconds(self):
        assert _seats().request_timeout == 60


class TestShouldRetry:
    def test_non_response_object_is_retried(self):
        assert _seats().should_retry("not-a-response") is True

    def test_401_not_retried(self):
        assert _seats().should_retry(real_response(status_code=401)) is False

    def test_plain_403_scope_denial_not_retried(self):
        resp = real_response(status_code=403, content=b"insufficient scope")
        assert _seats().should_retry(resp) is False

    def test_403_rate_limited_is_retried(self):
        resp = real_response(status_code=403, headers={"Retry-After": "5"})
        assert _seats().should_retry(resp) is True

    def test_404_not_retried(self):
        assert _seats().should_retry(real_response(status_code=404)) is False

    def test_409_not_retried(self):
        assert _seats().should_retry(real_response(status_code=409)) is False

    def test_204_not_retried(self):
        """No data for the day is a terminal, valid response — not a retry signal."""
        assert _seats().should_retry(real_response(status_code=204)) is False

    def test_429_is_retried(self):
        assert _seats().should_retry(real_response(status_code=429)) is True

    @pytest.mark.parametrize("code", [500, 502, 503, 504])
    def test_5xx_is_retried(self, code):
        assert _seats().should_retry(real_response(status_code=code)) is True

    def test_200_not_retried(self):
        assert _seats().should_retry(real_response(status_code=200)) is False


class TestBackoffTime:
    def test_non_response_defaults_to_sixty(self):
        assert _seats().backoff_time("not-a-response") == 60.0

    def test_429_uses_retry_after_header(self):
        resp = real_response(status_code=429, headers={"Retry-After": "12"})
        assert _seats().backoff_time(resp) == 12.0

    def test_403_rate_limited_uses_retry_after_header(self):
        resp = real_response(status_code=403, headers={"Retry-After": "9"})
        assert _seats().backoff_time(resp) == 9.0

    def test_429_without_retry_after_uses_ratelimit_reset(self, monkeypatch):
        import source_github_copilot.streams.base as base_module

        monkeypatch.setattr(base_module.time, "time", lambda: 1000.0)
        resp = real_response(status_code=429, headers={"X-RateLimit-Reset": "1010"})
        assert _seats().backoff_time(resp) == 11.0

    def test_502_is_fixed_sixty_seconds(self):
        assert _seats().backoff_time(real_response(status_code=502)) == 60.0

    def test_503_is_fixed_sixty_seconds(self):
        assert _seats().backoff_time(real_response(status_code=503)) == 60.0

    def test_other_codes_return_none(self):
        assert _seats().backoff_time(real_response(status_code=404)) is None


class TestGuardResponse:
    def test_401_raises_auth_error(self):
        with pytest.raises(CopilotAuthError):
            _seats()._guard_response(FakeResponse(status_code=401))

    def test_403_scope_denial_raises_auth_error(self):
        with pytest.raises(CopilotAuthError):
            _seats()._guard_response(FakeResponse(status_code=403, text="insufficient scope"))

    def test_403_rate_limited_does_not_raise_returns_false(self):
        resp = FakeResponse(status_code=403, headers={"Retry-After": "5"})
        assert _seats()._guard_response(resp) is False

    def test_204_returns_false_no_raise(self):
        assert _seats()._guard_response(FakeResponse(status_code=204)) is False

    def test_5xx_returns_false_logs_no_raise(self):
        assert _seats()._guard_response(FakeResponse(status_code=500, text="oops")) is False

    def test_2xx_returns_true(self):
        assert _seats()._guard_response(FakeResponse(status_code=200)) is True


class TestAddEnvelope:
    def test_injects_framework_fields(self):
        record = _seats()._add_envelope({"user_login": "alice"})
        assert record["tenant_id"] == "T"
        assert record["source_id"] == "S"
        assert record["data_source"] == "insight_github_copilot"
        assert "collected_at" in record

    def test_does_not_mutate_caller_dict(self):
        original = {"user_login": "alice"}
        _seats()._add_envelope(original)
        assert original == {"user_login": "alice"}

    def test_unique_key_composed_when_pk_parts_given(self):
        record = _seats()._add_envelope({"user_login": "alice"}, pk_parts=["alice"])
        assert record["unique_key"] == "T-S-alice"

    def test_no_unique_key_when_pk_parts_omitted(self):
        record = _seats()._add_envelope({"user_login": "alice"})
        assert "unique_key" not in record


class TestNextPageToken:
    def test_reports_stream_never_paginates_step_one(self):
        """The envelope call is one-per-day; Step-2 URL iteration happens inside
        parse_response, not via CDK pagination."""
        assert _reports().next_page_token(FakeResponse(status_code=200)) is None


class TestParseResponseEnvelopeBoundaries:
    """CopilotReportsStream.parse_response — Step-1 envelope handling."""

    def test_204_yields_nothing(self):
        assert list(_reports().parse_response(FakeResponse(status_code=204))) == []

    def test_non_json_envelope_yields_nothing_not_raises(self):
        resp = FakeResponse(status_code=200, json_error=ValueError("not json"))
        assert list(_reports().parse_response(resp)) == []

    def test_empty_download_links_yields_nothing(self):
        resp = FakeResponse(status_code=200, payload={"download_links": []})
        assert list(_reports().parse_response(resp)) == []

    def test_missing_download_links_key_yields_nothing(self):
        resp = FakeResponse(status_code=200, payload={})
        assert list(_reports().parse_response(resp)) == []


class TestFetchNdjsonRecords:
    def test_happy_path_parses_each_ndjson_line(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=200, lines=['{"id": 1}', '{"id": 2}']),
        )
        records = list(_reports()._fetch_ndjson_records("https://signed/x", "2026-01-01"))
        assert [r["id"] for r in records] == [1, 2]
        assert records[0]["unique_key"] == "T-S-1-2026-01-01"

    def test_blank_lines_skipped(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=200, lines=["", '{"id": 1}', ""]),
        )
        records = list(_reports()._fetch_ndjson_records("https://signed/x", "2026-01-01"))
        assert len(records) == 1

    def test_malformed_json_line_skipped_not_raised(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(
                status_code=200, lines=['{"id": 1}', "not-json{", '{"id": 2}']
            ),
        )
        records = list(_reports()._fetch_ndjson_records("https://signed/x", "2026-01-01"))
        assert [r["id"] for r in records] == [1, 2]

    def test_download_headers_used_no_authorization(self, monkeypatch):
        captured = {}

        def _fake_get(url, headers=None, **kw):
            captured["headers"] = headers
            return FakeResponse(status_code=200, lines=[])

        monkeypatch.setattr("source_github_copilot.streams.base.requests.get", _fake_get)
        list(_reports()._fetch_ndjson_records("https://signed/x", "2026-01-01"))
        assert "Authorization" not in captured["headers"]

    def test_network_error_treated_as_terminal_not_raised(self, monkeypatch):
        def _boom(*a, **kw):
            raise requests.ConnectionError("down")

        monkeypatch.setattr("source_github_copilot.streams.base.requests.get", _boom)
        assert list(_reports()._fetch_ndjson_records("https://signed/x", "2026-01-01")) == []

    def test_server_error_treated_as_terminal_not_raised(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=500),
        )
        assert list(_reports()._fetch_ndjson_records("https://signed/x", "2026-01-01")) == []

    def test_client_error_raises_signed_url_expired(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=403),
        )
        with pytest.raises(SignedUrlExpired):
            list(_reports()._fetch_ndjson_records("https://signed/x", "2026-01-01"))

    def test_filter_record_hook_drops_rejected_rows(self, monkeypatch):
        class _Filtered(_MinimalReportsStream):
            def _filter_record(self, record):
                return bool(record.get("keep"))

        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(
                status_code=200, lines=['{"id": 1, "keep": true}', '{"id": 2, "keep": false}']
            ),
        )
        records = list(_Filtered(**SHARED_STREAM_KWARGS)._fetch_ndjson_records("https://signed/x", "d"))
        assert [r["id"] for r in records] == [1]


class TestDownloadLinksEntryShapes:
    def test_plain_string_entries(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=200, lines=['{"id": 1}']),
        )
        resp = FakeResponse(status_code=200, payload={"download_links": ["https://signed/a"]})
        records = list(_reports().parse_response(resp, stream_slice={"day": "2026-01-01"}))
        assert len(records) == 1

    def test_dict_entries_with_url_key(self, monkeypatch):
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: FakeResponse(status_code=200, lines=['{"id": 1}']),
        )
        resp = FakeResponse(
            status_code=200, payload={"download_links": [{"url": "https://signed/a"}]}
        )
        records = list(_reports().parse_response(resp, stream_slice={"day": "2026-01-01"}))
        assert len(records) == 1

    def test_falsy_entry_skipped(self, monkeypatch):
        calls = []
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get",
            lambda *a, **kw: (calls.append(1), FakeResponse(status_code=200, lines=[]))[1],
        )
        resp = FakeResponse(status_code=200, payload={"download_links": ["", None]})
        records = list(_reports().parse_response(resp, stream_slice={"day": "2026-01-01"}))
        assert records == []
        assert calls == []  # neither falsy entry should trigger a Step-2 fetch

    def test_multiple_links_all_processed(self, monkeypatch):
        """Sharded payloads: every URL in download_links must be walked, not just the first."""
        pages = iter(
            [
                FakeResponse(status_code=200, lines=['{"id": 1}']),
                FakeResponse(status_code=200, lines=['{"id": 2}']),
            ]
        )
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get", lambda *a, **kw: next(pages)
        )
        resp = FakeResponse(
            status_code=200, payload={"download_links": ["https://signed/a", "https://signed/b"]}
        )
        records = list(_reports().parse_response(resp, stream_slice={"day": "2026-01-01"}))
        assert sorted(r["id"] for r in records) == [1, 2]


class TestSignedUrlExpiryRetryLoop:
    """The bounded refetch loop per DESIGN §3.6 — expiry mid-download triggers a
    fresh envelope fetch, capped at _SIGNED_URL_REFETCH_LIMIT (2)."""

    def test_recovers_after_one_expiry(self, monkeypatch):
        responses = iter(
            [
                FakeResponse(status_code=403),  # Step 2: expired signed URL
                FakeResponse(status_code=200, payload={"download_links": ["https://signed/new"]}),  # envelope refetch
                FakeResponse(status_code=200, lines=['{"id": 9}']),  # Step 2 on fresh URL
            ]
        )
        calls = []

        def _fake_get(*a, **kw):
            calls.append((a, kw))
            return next(responses)

        monkeypatch.setattr("source_github_copilot.streams.base.requests.get", _fake_get)
        initial = FakeResponse(status_code=200, payload={"download_links": ["https://signed/old"]})
        records = list(_reports().parse_response(initial, stream_slice={"day": "2026-01-01"}))
        assert [r["id"] for r in records] == [9]
        assert len(calls) == 3

    def test_gives_up_after_hitting_refetch_limit(self, monkeypatch):
        # Every attempt (initial + 2 refetches) expires again, so the loop must
        # stop after exactly _SIGNED_URL_REFETCH_LIMIT refetches — not spin forever.
        def _always_expired_cycle():
            while True:
                yield FakeResponse(status_code=403)  # Step 2 fetch — always expired
                yield FakeResponse(
                    status_code=200, payload={"download_links": ["https://signed/next"]}
                )  # envelope refetch always succeeds with another (also-expired) link

        gen = _always_expired_cycle()
        calls = []

        def _fake_get(*a, **kw):
            calls.append(1)
            return next(gen)

        monkeypatch.setattr("source_github_copilot.streams.base.requests.get", _fake_get)
        initial = FakeResponse(status_code=200, payload={"download_links": ["https://signed/old"]})
        records = list(_reports().parse_response(initial, stream_slice={"day": "2026-01-01"}))
        assert records == []
        # refetch_count starts at 0; loop stops once refetch_count reaches the
        # limit (2) — i.e. 3 signed-url attempts + 2 envelope refetches = 5 calls.
        assert len(calls) == 5

    def test_envelope_refetch_failure_stops_cleanly(self, monkeypatch):
        responses = iter(
            [
                FakeResponse(status_code=403),  # Step 2: expired
                FakeResponse(status_code=500),  # envelope refetch itself fails
            ]
        )
        monkeypatch.setattr(
            "source_github_copilot.streams.base.requests.get", lambda *a, **kw: next(responses)
        )
        initial = FakeResponse(status_code=200, payload={"download_links": ["https://signed/old"]})
        # Must not raise — a failed refetch is a clean stop, not a crash.
        records = list(_reports().parse_response(initial, stream_slice={"day": "2026-01-01"}))
        assert records == []

    def test_envelope_refetch_network_error_stops_cleanly(self, monkeypatch):
        calls = iter([FakeResponse(status_code=403)])  # Step 2: expired

        def _fake_get(*a, **kw):
            try:
                return next(calls)
            except StopIteration:
                raise requests.ConnectionError("dns fail")

        monkeypatch.setattr("source_github_copilot.streams.base.requests.get", _fake_get)
        initial = FakeResponse(status_code=200, payload={"download_links": ["https://signed/old"]})
        # The envelope refetch's own requests.get raising must not propagate —
        # _refetch_envelope catches RequestException and returns None.
        records = list(_reports().parse_response(initial, stream_slice={"day": "2026-01-01"}))
        assert records == []
