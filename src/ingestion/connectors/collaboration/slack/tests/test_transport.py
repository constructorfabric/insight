"""The client against a real HTTP server, so urllib3 does the transport decoding.

requests_mock bypasses urllib3: it cannot show that a file Slack sends with
Content-Encoding: gzip arrives already unpacked.
"""

from __future__ import annotations

import gzip
import json
import threading
from collections.abc import Iterator
from http.server import BaseHTTPRequestHandler, HTTPServer

import pytest
from source_slack import client as client_module
from source_slack.client import SlackAnalyticsClient, SlackApiError, SlackResponseError

_ROWS = [{"user_id": "U1", "date": "2026-10-01"}, {"user_id": "U2", "date": "2026-10-01"}]
_FILE = gzip.compress("\n".join(json.dumps(r) for r in _ROWS).encode())

_BOMB = gzip.compress(b"\n".join([json.dumps(_ROWS[0]).encode()] + [b" " * 1000] * 20_000))

_ANSWERS = {
    "bomb": (_BOMB, {"Content-Type": "application/gzip"}),
    "double": (gzip.compress(_FILE), {"Content-Type": "application/gzip", "Content-Encoding": "gzip"}),
    "members": (
        gzip.compress(json.dumps(_ROWS[0]).encode() + b"\n") + gzip.compress(json.dumps(_ROWS[1]).encode()),
        {"Content-Type": "application/gzip"},
    ),
    "truncated": (_FILE[:-12], {"Content-Type": "application/gzip"}),
    "corrupt": (_FILE[:10] + b"\x00" * 40, {"Content-Type": "application/gzip"}),
    "triple": (gzip.compress(gzip.compress(_FILE)), {"Content-Type": "application/gzip"}),
    "not-objects": (gzip.compress(b"[1, 2]\n"), {"Content-Type": "application/gzip"}),
    "long-complete-line": (
        gzip.compress(b'{"user_id": "' + b"x" * 5000 + b'"}\n' + json.dumps(_ROWS[0]).encode()),
        {"Content-Type": "application/gzip"},
    ),
    "newlines": (gzip.compress(b"\n" * (8 * 1024**2)), {"Content-Type": "application/gzip"}),
    "long-line": (gzip.compress(b'{"user_id": "' + b"x" * 5000 + b'"}'), {"Content-Type": "application/gzip"}),
    "file": (_FILE, {"Content-Type": "application/gzip"}),
    "encoded": (_FILE, {"Content-Type": "application/gzip", "Content-Encoding": "gzip"}),
    "encoded-one-row": (
        gzip.compress(json.dumps(_ROWS[0]).encode()),
        {"Content-Type": "application/gzip", "Content-Encoding": "gzip"},
    ),
    "refusal": (b'{"ok":false,"error":"file_not_found"}', {"Content-Type": "application/json; charset=utf-8"}),
}


class _Handler(BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 — BaseHTTPRequestHandler's hook name
        body, headers = _ANSWERS[self.path.rsplit("/", 1)[-1].split("?")[0]]
        self.send_response(200)
        for name, value in headers.items():
            self.send_header(name, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args: object) -> None:
        pass


@pytest.fixture
def server() -> Iterator[str]:
    httpd = HTTPServer(("127.0.0.1", 0), _Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{httpd.server_port}"
    httpd.shutdown()


@pytest.mark.parametrize("answer", ["file", "encoded"])
def test_reads_the_file_whether_or_not_the_transport_unpacked_it(
    server: str, answer: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(client_module, "API_URL", f"{server}/{answer}")

    assert list(SlackAnalyticsClient("xoxp-t").member_day("2026-10-01")) == _ROWS


def test_a_one_row_file_unpacked_by_the_transport_is_not_mistaken_for_a_refusal(
    server: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(client_module, "API_URL", f"{server}/encoded-one-row")

    assert list(SlackAnalyticsClient("xoxp-t").member_day("2026-10-01")) == _ROWS[:1]


def test_raises_the_refusal(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(client_module, "API_URL", f"{server}/refusal")

    with pytest.raises(SlackApiError) as raised:
        list(SlackAnalyticsClient("xoxp-t").member_day("2026-10-01"))
    assert raised.value.code == "file_not_found"


def _client_for(server: str, answer: str, monkeypatch: pytest.MonkeyPatch) -> SlackAnalyticsClient:
    monkeypatch.setattr(client_module, "API_URL", f"{server}/{answer}")
    return SlackAnalyticsClient("xoxp-t")


@pytest.mark.parametrize("answer", ["double", "members"])
def test_reads_a_double_layer_and_a_multi_member_gzip(
    server: str, answer: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    assert list(_client_for(server, answer, monkeypatch).member_day("2026-10-01")) == _ROWS


def test_stops_a_decompression_bomb_at_the_limit(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(client_module, "MAX_DECOMPRESSED_BYTES", 1024**2)
    rows = _client_for(server, "bomb", monkeypatch).member_day("2026-10-01")

    with pytest.raises(SlackResponseError, match="expands past"):
        list(rows)


def test_refuses_a_body_larger_than_the_compressed_limit(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(client_module, "MAX_COMPRESSED_BYTES", 100)

    with pytest.raises(SlackResponseError, match="exceeds 100"):
        list(_client_for(server, "bomb", monkeypatch).member_day("2026-10-01"))


def test_refuses_a_line_longer_than_the_limit(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(client_module, "MAX_LINE_BYTES", 1000)

    with pytest.raises(SlackResponseError, match="line exceeds"):
        list(_client_for(server, "long-line", monkeypatch).member_day("2026-10-01"))


@pytest.mark.parametrize(("answer", "match"), [("truncated", "ends mid-stream"), ("corrupt", "corrupt gzip")])
def test_refuses_a_broken_gzip(server: str, answer: str, match: str, monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(SlackResponseError, match=match):
        list(_client_for(server, answer, monkeypatch).member_day("2026-10-01"))


def test_rows_arrive_before_the_body_is_fully_read(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(client_module, "MAX_DECOMPRESSED_BYTES", 1024**2)
    rows = _client_for(server, "bomb", monkeypatch).member_day("2026-10-01")

    assert next(rows)["user_id"] == "U1"


def test_refuses_more_gzip_layers_than_slack_sends(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(SlackResponseError, match="gzip layers"):
        list(_client_for(server, "triple", monkeypatch).member_day("2026-10-01"))


def test_refuses_a_line_that_is_not_an_object(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    with pytest.raises(SlackResponseError, match="not a JSON object"):
        list(_client_for(server, "not-objects", monkeypatch).member_day("2026-10-01"))


def test_refuses_a_long_line_that_ends_inside_a_chunk(server: str, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(client_module, "MAX_LINE_BYTES", 1000)

    with pytest.raises(SlackResponseError, match="line exceeds"):
        list(_client_for(server, "long-complete-line", monkeypatch).member_day("2026-10-01"))


def test_a_body_of_bare_newlines_is_an_empty_file_bounded_by_the_cap(
    server: str, monkeypatch: pytest.MonkeyPatch
) -> None:
    assert list(_client_for(server, "newlines", monkeypatch).member_day("2026-10-01")) == []
    monkeypatch.setattr(client_module, "MAX_DECOMPRESSED_BYTES", 1024**2)
    with pytest.raises(SlackResponseError, match="expands past"):
        list(_client_for(server, "newlines", monkeypatch).member_day("2026-10-01"))
