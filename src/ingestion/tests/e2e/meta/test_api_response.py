"""Unit tests for ApiResponse + per-fixture request building.

These are pure-Python — no compose, no analytics-api binary. They exercise
the request shape and response normalization, which are independent of
whether the live binary is running.
"""

from __future__ import annotations

from pathlib import Path
from unittest.mock import MagicMock

import httpx
import pytest

from e2e_lib.analytics_api import AnalyticsApiProcess, ApiResponse
from e2e_lib.fixture_loader import Fixture, SpecYaml


pytestmark = pytest.mark.smoke


# ---------------------------------------------------------------------------
# ApiResponse.from_httpx — payload shape normalization
# ---------------------------------------------------------------------------


def _httpx_response(status: int, body) -> httpx.Response:
    return httpx.Response(
        status_code=status,
        json=body,
        request=httpx.Request("POST", "http://test/x"),
    )


def test_from_httpx_metric_query_shape() -> None:
    """`POST /v1/metrics/{id}/query` returns { items, page_info }."""
    r = _httpx_response(
        200,
        {"items": [{"x": 1}, {"x": 2}], "page_info": {"has_next": False, "cursor": None}},
    )
    out = ApiResponse.from_httpx(r)
    assert out.status_code == 200
    assert out.items == [{"x": 1}, {"x": 2}]
    assert out.page_info == {"has_next": False, "cursor": None}


def test_from_httpx_bare_list() -> None:
    """`GET /v1/metrics` returns a bare list — items normalized to the list."""
    r = _httpx_response(200, [{"id": "a"}, {"id": "b"}])
    out = ApiResponse.from_httpx(r)
    assert out.items == [{"id": "a"}, {"id": "b"}]
    assert out.page_info == {}


def test_from_httpx_error_keeps_status() -> None:
    """4xx/5xx responses still produce an ApiResponse — the caller decides what to do."""
    r = _httpx_response(404, {"type": "urn:insight:error:not_found", "detail": "no such metric"})
    out = ApiResponse.from_httpx(r)
    assert out.status_code == 404
    # No items in error body — left as []
    assert out.items == []
    assert out.raw["detail"] == "no such metric"


def test_from_httpx_empty_body() -> None:
    r = httpx.Response(status_code=204, request=httpx.Request("GET", "http://test/x"))
    out = ApiResponse.from_httpx(r)
    assert out.status_code == 204
    assert out.items == []
    assert out.raw is None


# ---------------------------------------------------------------------------
# call_fixture — request construction from Fixture
# ---------------------------------------------------------------------------


def _fake_proc(monkeypatch, response_body) -> tuple[AnalyticsApiProcess, list[dict]]:
    """Return a fake AnalyticsApiProcess that records every request without spawning."""
    proc = AnalyticsApiProcess.__new__(AnalyticsApiProcess)
    proc.base_url = "http://127.0.0.1:9999"
    captured: list[dict] = []

    class _FakeClient:
        def __init__(self, *a, **kw):
            pass

        def __enter__(self):
            return self

        def __exit__(self, *exc):
            return False

        def request(self, method, url, **kwargs):
            captured.append({"method": method, "url": url, **kwargs})
            return _httpx_response(200, response_body)

    monkeypatch.setattr(proc, "client", lambda: _FakeClient())
    return proc, captured


def _fixture(tmp_path: Path, *, metric_id: str | None = None, method: str = "POST", endpoint: str = "/v1/metrics/{metric_id}/query") -> Fixture:
    return Fixture(
        name="dummy",
        root=tmp_path,
        spec=SpecYaml(
            spec_version=1,
            endpoint=endpoint,
            request_body={"$top": 50, "$filter": "tenant_id eq 'x'"},
            dbt_selector="+silver_dummy+",
            key_columns=["id"],
            method=method,
            metric_id=metric_id,
        ),
        bronze_csvs=[],
    )


def test_call_fixture_post_with_body(monkeypatch, tmp_path: Path) -> None:
    proc, captured = _fake_proc(monkeypatch, {"items": [{"id": 1}]})
    fx = _fixture(tmp_path, metric_id="11111111-1111-1111-1111-111111111111")

    out = proc.call_fixture(fx)
    assert out.status_code == 200
    assert out.items == [{"id": 1}]
    assert len(captured) == 1
    call = captured[0]
    assert call["method"] == "POST"
    assert call["url"] == "/v1/metrics/11111111-1111-1111-1111-111111111111/query"
    assert call["json"] == {"$top": 50, "$filter": "tenant_id eq 'x'"}


def test_call_fixture_get_omits_body(monkeypatch, tmp_path: Path) -> None:
    proc, captured = _fake_proc(monkeypatch, [{"id": "a"}])
    fx = _fixture(tmp_path, endpoint="/v1/metrics", method="GET")
    fx.spec.__dict__["request_body"] = {}  # bypass frozen for test

    out = proc.call_fixture(fx)
    assert out.items == [{"id": "a"}]
    call = captured[0]
    assert call["method"] == "GET"
    assert call["url"] == "/v1/metrics"
    assert "json" not in call


def test_call_fixture_missing_metric_id_raises(tmp_path: Path) -> None:
    from e2e_lib.fixture_loader import FixtureError

    fx = _fixture(tmp_path, metric_id=None)
    with pytest.raises(FixtureError, match="metric_id"):
        fx.spec.resolved_endpoint()
