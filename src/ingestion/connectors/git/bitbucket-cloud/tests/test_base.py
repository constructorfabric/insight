from __future__ import annotations

import pytest

from source_bitbucket_cloud.streams.base import (
    _make_unique_key,
    _normalize_start_date,
    _now_iso,
    _truncate,
    _TRUNCATE_SUFFIX,
)
from tests.conftest import FakeResponse


# ---------------------------------------------------------------------------
# Module-level helpers
# ---------------------------------------------------------------------------


class TestNowIso:
    def test_format(self):
        value = _now_iso()
        assert len(value) == 20
        assert value.endswith("Z")
        assert value[4] == "-" and value[10] == "T"


class TestNormalizeStartDate:
    def test_none_and_empty_pass_through(self):
        assert _normalize_start_date(None) is None
        assert _normalize_start_date("") is None

    def test_bare_date_kept(self):
        assert _normalize_start_date("2026-06-30") == "2026-06-30"

    def test_full_iso_trimmed_to_date(self):
        assert _normalize_start_date("2026-06-30T12:34:56Z") == "2026-06-30"
        assert _normalize_start_date("2026-06-30T12:34:56+02:00") == "2026-06-30"

    def test_invalid_raises(self):
        with pytest.raises(ValueError):
            _normalize_start_date("not-a-date-x")
        with pytest.raises(ValueError):
            _normalize_start_date("2026-13-45")


class TestMakeUniqueKey:
    def test_joins_all_parts_with_colon(self):
        assert _make_unique_key("T", "S", "ws", "repo", "42") == "T:S:ws:repo:42"

    def test_no_extra_parts(self):
        assert _make_unique_key("T", "S") == "T:S:"


class TestTruncate:
    def test_none_stays_none(self):
        assert _truncate(None) is None

    def test_short_text_passthrough(self):
        assert _truncate("hello") == "hello"

    def test_long_text_capped_with_suffix(self):
        out = _truncate("x" * 5000)
        assert out.endswith(_TRUNCATE_SUFFIX)
        assert len(out.encode("utf-8")) <= 1024

    def test_multibyte_boundary_not_broken(self):
        out = _truncate("я" * 3000)
        assert out.endswith(_TRUNCATE_SUFFIX)
        out.encode("utf-8")  # must be valid UTF-8

    def test_limit_smaller_than_suffix(self):
        out = _truncate("hello world", limit=3)
        assert len(out.encode("utf-8")) <= 3


# ---------------------------------------------------------------------------
# BitbucketCloudStream methods (via the concrete repositories stream fixture)
# ---------------------------------------------------------------------------


class TestRequestBasics:
    def test_request_headers_carry_auth(self, repositories_stream):
        headers = repositories_stream.request_headers()
        assert headers["Authorization"] == "Bearer tok"

    def test_base_request_params_pagelen_only(self, pr_comments_stream):
        # pr_comments inherits the base request_params (no override).
        assert pr_comments_stream.request_params() == {"pagelen": "100"}

    def test_next_page_token_none_without_next(self, repositories_stream):
        assert repositories_stream.next_page_token(FakeResponse({"values": []})) is None

    def test_next_page_token_wraps_next_url(self, repositories_stream):
        nxt = "https://api.bitbucket.org/2.0/repositories/ws?page=2"
        token = repositories_stream.next_page_token(FakeResponse({"next": nxt}))
        assert token == {"next_url": nxt}

    def test_next_page_token_tolerates_non_json(self, repositories_stream):
        resp = FakeResponse(ValueError("boom"))
        assert repositories_stream.next_page_token(resp) is None


class TestPathResolution:
    def test_path_uses_slice_when_no_token(self, repositories_stream):
        path = repositories_stream.path(stream_slice={"workspace": "ws"})
        assert path == "repositories/ws"

    def test_path_strips_url_base_from_next_url(self, repositories_stream):
        token = {"next_url": "https://api.bitbucket.org/2.0/repositories/ws?page=2"}
        assert repositories_stream.path(next_page_token=token) == "repositories/ws?page=2"

    def test_path_foreign_next_url_falls_back_to_replace(self, repositories_stream):
        token = {"next_url": "https://elsewhere.example/repositories/ws"}
        assert repositories_stream.path(next_page_token=token) == (
            "https://elsewhere.example/repositories/ws"
        )

    def test_base_path_not_implemented(self):
        from source_bitbucket_cloud.streams.base import BitbucketCloudStream

        class Minimal(BitbucketCloudStream):
            name = "minimal"

            def parse_response(self, response, **kwargs):  # pragma: no cover
                yield from ()

        stream = Minimal(token="t", tenant_id="T", source_id="S")
        with pytest.raises(NotImplementedError):
            stream.path(stream_slice={})


class TestRetryPolicy:
    @pytest.mark.parametrize("code", [401, 403, 404])
    def test_terminal_codes_not_retried(self, repositories_stream, code):
        resp = FakeResponse({}, status_code=code)
        resp.text = "denied"
        assert repositories_stream.should_retry(resp) is False

    @pytest.mark.parametrize("code", [429, 500, 502, 503, 504])
    def test_retryable_codes(self, repositories_stream, code):
        resp = FakeResponse({}, status_code=code)
        assert repositories_stream.should_retry(resp) is True

    def test_200_not_retried(self, repositories_stream):
        assert repositories_stream.should_retry(FakeResponse({}, status_code=200)) is False


class TestBackoff:
    def _resp(self, code, retry_after=None):
        resp = FakeResponse({}, status_code=code)
        resp.headers = {"Retry-After": retry_after} if retry_after is not None else {}
        return resp

    def test_429_honours_retry_after(self, repositories_stream):
        assert repositories_stream.backoff_time(self._resp(429, "17")) == 17.0

    def test_429_clamps_to_min_1s(self, repositories_stream):
        assert repositories_stream.backoff_time(self._resp(429, "0")) == 1.0

    def test_429_default_60s_without_header(self, repositories_stream):
        assert repositories_stream.backoff_time(self._resp(429)) == 60.0

    def test_429_garbage_header_falls_back(self, repositories_stream):
        assert repositories_stream.backoff_time(self._resp(429, "soon")) == 60.0

    def test_5xx_honours_retry_after(self, repositories_stream):
        assert repositories_stream.backoff_time(self._resp(503, "42")) == 42.0

    def test_5xx_without_header_uses_30s_plus_jitter(self, repositories_stream):
        wait = repositories_stream.backoff_time(self._resp(500))
        assert 30.0 <= wait <= 40.0

    def test_5xx_garbage_header_uses_30s_plus_jitter(self, repositories_stream):
        wait = repositories_stream.backoff_time(self._resp(502, "later"))
        assert 30.0 <= wait <= 40.0

    def test_other_codes_no_backoff(self, repositories_stream):
        assert repositories_stream.backoff_time(self._resp(404)) is None


class TestErrorHandler:
    def test_default_handler_without_ignore_404(self, repositories_stream):
        assert repositories_stream.ignore_404 is False
        assert repositories_stream.get_error_handler() is not None

    def test_ignore_404_handler_maps_404_to_ignore(self, branches_stream):
        from airbyte_cdk.sources.streams.http.error_handlers.response_models import (
            ResponseAction,
        )

        assert branches_stream.ignore_404 is True
        handler = branches_stream.get_error_handler()
        resolution = handler._error_mapping[404]
        assert resolution.response_action == ResponseAction.IGNORE


class TestEnvelopeAndIterValues:
    def test_envelope_adds_tenant_fields(self, repositories_stream):
        out = repositories_stream._envelope({"a": 1})
        assert out["a"] == 1
        assert out["tenant_id"] == "T"
        assert out["source_id"] == "S"
        assert out["data_source"] == "insight_bitbucket_cloud"
        assert out["collected_at"].endswith("Z")

    def test_iter_values_returns_values(self, repositories_stream):
        vals = list(repositories_stream._iter_values(FakeResponse({"values": [{"x": 1}]})))
        assert vals == [{"x": 1}]

    def test_iter_values_empty_and_null(self, repositories_stream):
        assert list(repositories_stream._iter_values(FakeResponse({}))) == []
        assert list(repositories_stream._iter_values(FakeResponse({"values": None}))) == []

    def test_iter_values_non_json_returns_empty(self, repositories_stream):
        resp = FakeResponse(ValueError("not json"))
        assert list(repositories_stream._iter_values(resp)) == []
