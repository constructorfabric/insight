from __future__ import annotations

import time

import pytest
import requests
import source_github_v2.streams.base as base
from source_github_v2.streams.base import (
    GitHubAuthError,
    GitHubGraphQLStream,
    GitHubRestStream,
    _is_rate_limit_403,
    _make_unique_key,
    _now_iso,
)
from tests.conftest import SHARED, FakeResponse


class MinimalRest(GitHubRestStream):
    """Concrete REST stream exercising the base parse_response/_path contract."""

    name = "minimal_rest"

    def _path(self, **kwargs) -> str:
        return "rest/endpoint"


class MinimalGraphQL(GitHubGraphQLStream):
    """Concrete GraphQL stream with a flat data.items connection."""

    name = "minimal_graphql"

    def _query(self) -> str:
        return "query { items }"

    def _variables(self, stream_slice=None, next_page_token=None) -> dict:
        variables = {"slice": stream_slice}
        if next_page_token:
            variables["after"] = next_page_token.get("after")
        return variables

    def _extract_nodes(self, data: dict) -> list:
        return self._safe_get(data, "items", "nodes") or []

    def _extract_page_info(self, data: dict) -> dict:
        return self._safe_get(data, "items", "pageInfo") or {}


@pytest.fixture
def rest_stream() -> MinimalRest:
    return MinimalRest(**SHARED)


@pytest.fixture
def gql_stream() -> MinimalGraphQL:
    return MinimalGraphQL(**SHARED)


# ---------------------------------------------------------------------------
# Module-level helpers
# ---------------------------------------------------------------------------


class TestNowIso:
    def test_format(self):
        value = _now_iso()
        assert len(value) == 20
        assert value.endswith("Z")
        assert value[4] == "-" and value[10] == "T"


class TestMakeUniqueKey:
    def test_joins_all_parts_with_colon(self):
        assert _make_unique_key("T", "S", "acme", "repo", "42") == "T:S:acme:repo:42"

    def test_no_extra_parts(self):
        assert _make_unique_key("T", "S") == "T:S:"


class TestIsRateLimit403:
    def test_non_403_never_rate_limit(self):
        assert _is_rate_limit_403(FakeResponse({}, status_code=429)) is False

    def test_retry_after_header(self):
        resp = FakeResponse({}, status_code=403, headers={"Retry-After": "5"})
        assert _is_rate_limit_403(resp) is True

    def test_remaining_zero_header(self):
        resp = FakeResponse({}, status_code=403, headers={"X-RateLimit-Remaining": "0"})
        assert _is_rate_limit_403(resp) is True

    def test_body_mentions_rate_limit(self):
        resp = FakeResponse({}, status_code=403, text="You have hit a secondary rate limit")
        assert _is_rate_limit_403(resp) is True

    def test_plain_403_is_auth(self):
        resp = FakeResponse({}, status_code=403, text="forbidden")
        assert _is_rate_limit_403(resp) is False

    def test_unreadable_body_treated_as_auth(self):
        class NoText:
            status_code = 403
            headers: dict = {}

            @property
            def text(self):
                raise RuntimeError("stream consumed")

        assert _is_rate_limit_403(NoText()) is False


# ---------------------------------------------------------------------------
# GitHubRestStream
# ---------------------------------------------------------------------------


class TestRestRequestBasics:
    def test_request_headers_carry_auth(self, rest_stream):
        headers = rest_stream.request_headers()
        assert headers["Authorization"] == "Bearer tok"
        assert headers["Accept"] == "application/vnd.github+json"

    def test_default_params(self, rest_stream):
        assert rest_stream.request_params() == {"per_page": "100"}

    def test_request_timeout(self, rest_stream):
        assert rest_stream.request_timeout == 60

    def test_next_page_token_from_link_header(self, rest_stream):
        nxt = "https://api.github.com/orgs/acme/repos?page=2"
        resp = FakeResponse({}, links={"next": {"url": nxt}})
        assert rest_stream.next_page_token(resp) == {"next_url": nxt}

    def test_next_page_token_none_without_link(self, rest_stream):
        assert rest_stream.next_page_token(FakeResponse({})) is None

    def test_path_delegates_to_subclass(self, rest_stream):
        assert rest_stream.path() == "rest/endpoint"

    def test_path_strips_url_base_from_next_url(self, rest_stream):
        token = {"next_url": "https://api.github.com/orgs/acme/repos?page=2"}
        assert rest_stream.path(next_page_token=token) == "orgs/acme/repos?page=2"


class TestRestRetryPolicy:
    def test_connection_error_always_retried(self, rest_stream):
        assert rest_stream.should_retry(object()) is True

    def test_rate_limited_403_retried(self, rest_stream):
        resp = FakeResponse({}, status_code=403, headers={"Retry-After": "5"})
        assert rest_stream.should_retry(resp) is True

    @pytest.mark.parametrize("code", [401, 403, 404, 409])
    def test_terminal_codes_not_retried(self, rest_stream, code):
        assert rest_stream.should_retry(FakeResponse({}, status_code=code)) is False

    @pytest.mark.parametrize("code", [429, 500, 502, 503, 504])
    def test_retryable_codes(self, rest_stream, code):
        assert rest_stream.should_retry(FakeResponse({}, status_code=code)) is True

    def test_200_not_retried(self, rest_stream):
        assert rest_stream.should_retry(FakeResponse({}, status_code=200)) is False


class TestRestBackoff:
    def test_connection_error_60s(self, rest_stream):
        assert rest_stream.backoff_time(object()) == 60.0

    def test_429_honours_retry_after(self, rest_stream):
        resp = FakeResponse({}, status_code=429, headers={"Retry-After": "17"})
        assert rest_stream.backoff_time(resp) == 17.0

    def test_429_clamps_to_min_1s(self, rest_stream):
        resp = FakeResponse({}, status_code=429, headers={"Retry-After": "0"})
        assert rest_stream.backoff_time(resp) == 1.0

    def test_429_uses_ratelimit_reset(self, rest_stream):
        reset = str(time.time() + 30)
        resp = FakeResponse({}, status_code=429, headers={"X-RateLimit-Reset": reset})
        wait = rest_stream.backoff_time(resp)
        assert 25.0 <= wait <= 35.0

    def test_rate_limited_403_with_reset_in_past_clamps(self, rest_stream):
        resp = FakeResponse(
            {}, status_code=403, headers={"X-RateLimit-Remaining": "0", "X-RateLimit-Reset": str(time.time() - 100)}
        )
        assert rest_stream.backoff_time(resp) == 1.0

    @pytest.mark.parametrize("code", [502, 503])
    def test_5xx_fixed_60s(self, rest_stream, code):
        assert rest_stream.backoff_time(FakeResponse({}, status_code=code)) == 60.0

    def test_other_codes_no_backoff(self, rest_stream):
        assert rest_stream.backoff_time(FakeResponse({}, status_code=404)) is None


class TestRestGuardAndParse:
    def test_401_raises_auth_error(self, rest_stream):
        with pytest.raises(GitHubAuthError):
            rest_stream._guard_response(FakeResponse({}, status_code=401, text="bad token"))

    def test_plain_403_raises_auth_error(self, rest_stream):
        with pytest.raises(GitHubAuthError):
            rest_stream._guard_response(FakeResponse({}, status_code=403, text="forbidden"))

    def test_rate_limited_403_does_not_raise(self, rest_stream):
        resp = FakeResponse({}, status_code=403, headers={"Retry-After": "5"})
        assert rest_stream._guard_response(resp) is False

    def test_404_logged_not_raised(self, rest_stream):
        assert rest_stream._guard_response(FakeResponse({}, status_code=404)) is False

    def test_200_ok(self, rest_stream):
        assert rest_stream._guard_response(FakeResponse({})) is True

    def test_parse_response_list_payload(self, rest_stream):
        records = list(rest_stream.parse_response(FakeResponse([{"a": 1}, {"a": 2}])))
        assert [r["a"] for r in records] == [1, 2]
        assert records[0]["tenant_id"] == "T"
        assert records[0]["data_source"] == "insight_github"

    def test_parse_response_dict_payload_wrapped(self, rest_stream):
        records = list(rest_stream.parse_response(FakeResponse({"a": 1})))
        assert len(records) == 1 and records[0]["a"] == 1

    def test_parse_response_stops_on_error_status(self, rest_stream):
        assert list(rest_stream.parse_response(FakeResponse({}, status_code=404))) == []

    def test_envelope_with_pk_parts(self, rest_stream):
        out = rest_stream._add_envelope({"a": 1}, pk_parts=["x", "y"])
        assert out["unique_key"] == "T:S:x:y"
        assert out["collected_at"].endswith("Z")

    def test_envelope_does_not_mutate_input(self, rest_stream):
        original = {"a": 1}
        rest_stream._add_envelope(original)
        assert original == {"a": 1}


# ---------------------------------------------------------------------------
# GitHubGraphQLStream
# ---------------------------------------------------------------------------


class TestGraphQLRequestBasics:
    def test_path_is_graphql(self, gql_stream):
        assert gql_stream.path() == "graphql"

    def test_request_timeout(self, gql_stream):
        assert gql_stream.request_timeout == 120

    def test_headers_are_plain_bearer(self, gql_stream):
        headers = gql_stream.request_headers()
        assert headers["Authorization"] == "Bearer tok"
        assert "Accept" not in headers

    def test_request_body_json(self, gql_stream):
        body = gql_stream.request_body_json(stream_slice={"x": 1}, next_page_token={"after": "c1"})
        assert body["query"] == "query { items }"
        assert body["variables"] == {"slice": {"x": 1}, "after": "c1"}

    def test_safe_get_tolerates_none(self, gql_stream):
        assert gql_stream._safe_get({"a": None}, "a", "b") is None
        assert gql_stream._safe_get({"a": {"b": 3}}, "a", "b") == 3

    def test_next_page_token_follows_page_info(self, gql_stream):
        body = {"data": {"items": {"pageInfo": {"hasNextPage": True, "endCursor": "c9"}}}}
        assert gql_stream.next_page_token(FakeResponse(body)) == {"after": "c9"}

    def test_next_page_token_none_when_done(self, gql_stream):
        body = {"data": {"items": {"pageInfo": {"hasNextPage": False, "endCursor": None}}}}
        assert gql_stream.next_page_token(FakeResponse(body)) is None


class TestGraphQLRateLimitDetection:
    def test_rate_limited_error_type(self, gql_stream):
        body = {"errors": [{"type": "RATE_LIMITED", "message": "slow down"}]}
        assert gql_stream._is_graphql_rate_limited(FakeResponse(body)) is True

    def test_rate_limit_in_message(self, gql_stream):
        body = {"errors": [{"message": "API rate limit exceeded"}]}
        assert gql_stream._is_graphql_rate_limited(FakeResponse(body)) is True

    def test_other_errors_not_rate_limit(self, gql_stream):
        body = {"errors": [{"type": "NOT_FOUND", "message": "missing"}]}
        assert gql_stream._is_graphql_rate_limited(FakeResponse(body)) is False

    def test_non_200_not_checked(self, gql_stream):
        body = {"errors": [{"type": "RATE_LIMITED"}]}
        assert gql_stream._is_graphql_rate_limited(FakeResponse(body, status_code=502)) is False

    def test_unparseable_body_ignored(self, gql_stream):
        resp = FakeResponse(ValueError("not json"))
        assert gql_stream._is_graphql_rate_limited(resp) is False


class TestGraphQLRetryPolicy:
    def test_connection_error_always_retried(self, gql_stream):
        assert gql_stream.should_retry(object()) is True

    def test_body_rate_limit_retried(self, gql_stream):
        body = {"errors": [{"type": "RATE_LIMITED"}]}
        assert gql_stream.should_retry(FakeResponse(body)) is True

    def test_rate_limited_403_retried(self, gql_stream):
        resp = FakeResponse({}, status_code=403, headers={"Retry-After": "5"})
        assert gql_stream.should_retry(resp) is True

    @pytest.mark.parametrize("code", [401, 403])
    def test_auth_codes_not_retried(self, gql_stream, code):
        assert gql_stream.should_retry(FakeResponse({}, status_code=code)) is False

    @pytest.mark.parametrize("code", [429, 500, 502, 503, 504])
    def test_retryable_codes(self, gql_stream, code):
        assert gql_stream.should_retry(FakeResponse({}, status_code=code)) is True

    def test_clean_200_not_retried(self, gql_stream):
        assert gql_stream.should_retry(FakeResponse({"data": {}})) is False


class TestGraphQLBackoff:
    def test_connection_error_60s(self, gql_stream):
        assert gql_stream.backoff_time(object()) == 60.0

    def test_body_rate_limit_uses_reset_header(self, gql_stream):
        body = {"errors": [{"type": "RATE_LIMITED"}]}
        reset = str(time.time() + 30)
        resp = FakeResponse(body, headers={"x-ratelimit-reset": reset})
        wait = gql_stream.backoff_time(resp)
        assert 25.0 <= wait <= 35.0

    def test_body_rate_limit_default_60s(self, gql_stream):
        body = {"errors": [{"type": "RATE_LIMITED"}]}
        assert gql_stream.backoff_time(FakeResponse(body)) == 60.0

    def test_429_honours_retry_after(self, gql_stream):
        resp = FakeResponse({"data": {}}, status_code=429, headers={"Retry-After": "9"})
        assert gql_stream.backoff_time(resp) == 9.0

    def test_429_uses_reset_header(self, gql_stream):
        reset = str(time.time() + 20)
        resp = FakeResponse({"data": {}}, status_code=429, headers={"x-ratelimit-reset": reset})
        wait = gql_stream.backoff_time(resp)
        assert 15.0 <= wait <= 25.0

    def test_429_default_60s(self, gql_stream):
        assert gql_stream.backoff_time(FakeResponse({"data": {}}, status_code=429)) == 60.0

    @pytest.mark.parametrize("code", [502, 503])
    def test_5xx_fixed_60s(self, gql_stream, code):
        assert gql_stream.backoff_time(FakeResponse({"data": {}}, status_code=code)) == 60.0

    def test_clean_200_no_backoff(self, gql_stream):
        assert gql_stream.backoff_time(FakeResponse({"data": {}})) is None


class TestGraphQLParseResponse:
    def _body(self, nodes, rate_limit=None):
        data = {"items": {"nodes": nodes, "pageInfo": {"hasNextPage": False}}}
        if rate_limit is not None:
            data["rateLimit"] = rate_limit
        return {"data": data}

    def test_yields_enveloped_nodes(self, gql_stream):
        records = list(gql_stream.parse_response(FakeResponse(self._body([{"id": 1}]))))
        assert len(records) == 1
        assert records[0]["id"] == 1
        assert records[0]["tenant_id"] == "T"

    def test_errors_without_data_raise(self, gql_stream):
        body = {"errors": [{"message": "boom"}]}
        with pytest.raises(RuntimeError, match="GraphQL query failed"):
            list(gql_stream.parse_response(FakeResponse(body)))

    def test_partial_errors_still_emit_data(self, gql_stream):
        body = self._body([{"id": 1}])
        body["errors"] = [{"message": "partial"}]
        records = list(gql_stream.parse_response(FakeResponse(body)))
        assert len(records) == 1

    def test_rate_limit_info_logged(self, gql_stream, caplog):
        body = self._body([], rate_limit={"cost": 1, "remaining": 42})
        with caplog.at_level("WARNING", logger="airbyte"):
            list(gql_stream.parse_response(FakeResponse(body)))
        assert "rate limit low" in caplog.text

    def test_envelope_with_pk_parts(self, gql_stream):
        out = gql_stream._add_envelope({"a": 1}, pk_parts=["x", "y"])
        assert out["unique_key"] == "T:S:x:y"
        assert out["tenant_id"] == "T"


class TestSendGraphQL:
    """_send_graphql drives requests.post directly — stub it at module level."""

    @pytest.fixture(autouse=True)
    def no_sleep(self, monkeypatch):
        monkeypatch.setattr(base.time, "sleep", lambda _s: None)

    def _post_stub(self, monkeypatch, responses):
        calls = []

        def fake_post(url, json=None, headers=None, timeout=None):
            calls.append({"url": url, "json": json, "headers": headers})
            result = responses.pop(0)
            if isinstance(result, Exception):
                raise result
            return result

        monkeypatch.setattr(base.requests, "post", fake_post)
        return calls

    def test_success_first_try(self, gql_stream, monkeypatch):
        self._post_stub(monkeypatch, [FakeResponse({"data": {"ok": True}})])
        body = gql_stream._send_graphql("q", {"v": 1})
        assert body == {"data": {"ok": True}}

    def test_sends_query_and_variables(self, gql_stream, monkeypatch):
        calls = self._post_stub(monkeypatch, [FakeResponse({"data": {}})])
        gql_stream._send_graphql("query Q", {"a": 2})
        assert calls[0]["url"] == "https://api.github.com/graphql"
        assert calls[0]["json"] == {"query": "query Q", "variables": {"a": 2}}
        assert calls[0]["headers"]["Authorization"] == "Bearer tok"

    def test_retryable_then_success(self, gql_stream, monkeypatch):
        self._post_stub(monkeypatch, [FakeResponse({}, status_code=500), FakeResponse({"data": {"ok": 1}})])
        assert gql_stream._send_graphql("q", {}) == {"data": {"ok": 1}}

    def test_connection_error_then_success(self, gql_stream, monkeypatch):
        self._post_stub(monkeypatch, [requests.ConnectionError("refused"), FakeResponse({"data": {}})])
        assert gql_stream._send_graphql("q", {}) == {"data": {}}

    def test_connection_errors_exhaust_retries(self, gql_stream, monkeypatch):
        self._post_stub(monkeypatch, [requests.ConnectionError("refused")] * 3)
        with pytest.raises(requests.ConnectionError):
            gql_stream._send_graphql("q", {}, max_retries=2)

    def test_retryable_exhausts_retries(self, gql_stream, monkeypatch):
        self._post_stub(monkeypatch, [FakeResponse({}, status_code=502, text="bad gw")] * 2)
        with pytest.raises(RuntimeError, match="failed after 2 attempts"):
            gql_stream._send_graphql("q", {}, max_retries=1)

    def test_non_retryable_error_status_raises(self, gql_stream, monkeypatch):
        self._post_stub(monkeypatch, [FakeResponse({}, status_code=400, text="bad query")])
        with pytest.raises(RuntimeError, match="400"):
            gql_stream._send_graphql("q", {})

    def test_graphql_errors_without_data_raise(self, gql_stream, monkeypatch):
        self._post_stub(monkeypatch, [FakeResponse({"errors": [{"message": "nope"}]})])
        with pytest.raises(RuntimeError, match="GraphQL query failed"):
            gql_stream._send_graphql("q", {})
