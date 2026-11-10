from __future__ import annotations

import http.client
import json
from http.server import BaseHTTPRequestHandler

import pytest
from proxy_client import request, rule, running, wait_event

PAYLOAD = b'{"job":{"status":"succeeded"}}'


class FramedUpstream(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format: str, *args: object) -> None:
        return

    def do_GET(self) -> None:
        self.close_connection = True
        self.send_response(self.server.response_status)
        if self.server.response_status in {204, 304}:
            self.end_headers()
            return
        if self.server.framing == "chunked":
            self.send_header("Transfer-Encoding", "chunked")
        else:
            self.send_header("Content-Length", str(len(PAYLOAD) + int(self.server.failure != "none")))
        self.end_headers()
        if self.command == "HEAD":
            return
        wire = f"{len(PAYLOAD):X}\r\n".encode() + PAYLOAD + b"\r\n"
        self.wfile.write(wire if self.server.framing == "chunked" else PAYLOAD)
        self.wfile.flush()
        if self.server.failure == "timeout":
            self.server.finish.wait(3)
        elif self.server.failure == "none" and self.server.framing == "chunked":
            self.wfile.write(b"0\r\n\r\n")

    do_HEAD = do_GET


@pytest.mark.parametrize("method", ["GET", "HEAD"])
def test_bodyless_requests_do_not_send_empty_payloads(monkeypatch: pytest.MonkeyPatch, method: str) -> None:
    original = http.client.HTTPConnection.send

    def reject_empty_send(connection: http.client.HTTPConnection, data: object) -> None:
        if data == b"":
            raise BrokenPipeError("synthetic upstream closed after reading headers")
        original(connection, data)

    monkeypatch.setattr(http.client.HTTPConnection, "send", reject_empty_send)
    with running(FramedUpstream) as (proxy, upstream):
        upstream.framing = "length"
        upstream.failure = "none"
        upstream.response_status = 200
        status, body = request(proxy, method, "/api/v1/jobs")
        assert status == 200
        assert body == (b"" if method == "HEAD" else PAYLOAD)


@pytest.mark.parametrize("framing", ["chunked", "length"])
@pytest.mark.parametrize("failure", ["eof", "timeout"])
def test_incomplete_upstream_after_valid_json_is_incomplete_downstream(framing: str, failure: str) -> None:
    with running(FramedUpstream) as (proxy, upstream):
        upstream.framing = framing
        upstream.failure = failure
        upstream.response_status = 200
        proxy.upstream_timeout = 0.05
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=2)
        try:
            client.request("GET", "/api/v1/jobs")
            response = client.getresponse()
            assert response.status == 200
            with pytest.raises(http.client.IncompleteRead) as caught:
                response.read()
            assert caught.value.partial == PAYLOAD
            wait_event(proxy.state, "request_finished")
            events = proxy.state.observations(0)["events"]
            assert any(row["event"] == "transport_failure" for row in events)
            assert any(row["event"] == "request_finished" and row["outcome"] == "failed" for row in events)
            assert not any(row["event"] == "response" for row in events)
        finally:
            client.close()


@pytest.mark.parametrize("framing", ["chunked", "length"])
@pytest.mark.parametrize("rewritten", [False, True])
def test_complete_bodies_have_unambiguous_framing(framing: str, rewritten: bool) -> None:
    with running(FramedUpstream) as (proxy, upstream):
        upstream.framing = framing
        upstream.failure = "none"
        upstream.response_status = 200
        if rewritten:
            proxy.state.configure({"rules": [rule({"type": "rewrite", "field": ["job", "status"], "value": "failed"})]})
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=2)
        try:
            client.request("GET", "/api/v1/jobs")
            response = client.getresponse()
            body = response.read()
            assert response.status == 200
            assert json.loads(body) == {"job": {"status": "failed" if rewritten else "succeeded"}}
            assert response.getheader("Transfer-Encoding") == (None if rewritten else "chunked")
            assert response.getheader("Content-Length") == (str(len(body)) if rewritten else None)
        finally:
            client.close()


@pytest.mark.parametrize("framing", ["chunked", "length"])
@pytest.mark.parametrize("failure", ["eof", "timeout"])
def test_rewrites_refuse_incomplete_upstream_json(framing: str, failure: str) -> None:
    with running(FramedUpstream) as (proxy, upstream):
        upstream.framing = framing
        upstream.failure = failure
        upstream.response_status = 200
        proxy.upstream_timeout = 0.05
        proxy.state.configure({"rules": [rule({"type": "rewrite", "field": ["job", "status"], "value": "failed"})]})
        assert request(proxy, "GET", "/api/v1/jobs")[0] == 502


@pytest.mark.parametrize("method,status", [("HEAD", 200), ("GET", 204), ("GET", 304)])
def test_bodyless_responses_have_no_chunked_body(method: str, status: int) -> None:
    with running(FramedUpstream) as (proxy, upstream):
        upstream.framing = "length"
        upstream.failure = "none"
        upstream.response_status = status
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=2)
        try:
            client.request(method, "/api/v1/jobs")
            response = client.getresponse()
            assert response.status == status
            assert response.getheader("Transfer-Encoding") is None
            assert response.read() == b""
        finally:
            client.close()
