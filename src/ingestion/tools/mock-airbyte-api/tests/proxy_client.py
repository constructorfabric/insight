from __future__ import annotations

import http.client
import json
import socket
import threading
import time
from collections.abc import Iterator
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch

import pytest
from proxy_rules import MAX_REWRITE_BYTES
from proxy_server import Server
from proxy_state import State

CONNECTION = "00000000-0000-0000-0000-000000000001"
TOKEN = "synthetic-secret-control"


def rule(action: dict[str, object], **match: object) -> dict[str, object]:
    return {
        "name": "fault",
        "method": "*",
        "path": "/api/*",
        "connection_id": None,
        "job_id": None,
        "call": None,
        "action": action,
        **match,
    }


class Upstream(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, format: str, *args: object) -> None:
        return

    def do_POST(self) -> None:
        if self.headers.get("Transfer-Encoding") == "chunked":
            chunks = bytearray()
            wire = bytearray()
            while True:
                line = self.rfile.readline()
                wire.extend(line)
                size = int(line.split(b";", 1)[0], 16)
                if not size:
                    while True:
                        trailer = self.rfile.readline()
                        wire.extend(trailer)
                        if trailer == b"\r\n":
                            break
                    break
                chunk = self.rfile.read(size)
                chunks.extend(chunk)
                wire.extend(chunk + self.rfile.read(2))
            body = bytes(chunks)
            self.server.chunked_wire = bytes(wire)
        else:
            body = self.rfile.read(int(self.headers["Content-Length"]))
        self.server.received.append((self.command, self.path, list(self.headers.items()), body))
        payload = json.dumps({"job": {"id": 42, "status": "succeeded", "streamStats": [1]}}).encode()
        time.sleep(self.server.header_delay)
        self.send_response(201)
        self.send_header("Content-Length", str(len(payload)))
        self.send_header("X-Synthetic", "kept")
        self.end_headers()
        if self.path.endswith("/slow"):
            self.server.finish.wait(3)
        time.sleep(self.server.body_delay)
        try:
            self.wfile.write(payload)
        except OSError:
            pass

    def do_GET(self) -> None:
        self.server.received.append((self.command, self.path, list(self.headers.items()), b""))
        self.send_response(200)
        if self.path.endswith("/oversize"):
            self.send_header("Content-Length", str(MAX_REWRITE_BYTES + 1))
            self.end_headers()
            try:
                self.wfile.write(
                    b'{"job":{"status":"succeeded"}}'
                    + b" " * (MAX_REWRITE_BYTES + 1 - len(b'{"job":{"status":"succeeded"}}'))
                )
            except OSError:
                pass
            return
        if self.path.endswith("/large"):
            first = b"a" * 65536
            self.send_header("Content-Length", str(len(first) * 80))
            self.end_headers()
            self.wfile.write(first)
            self.wfile.flush()
            self.server.first_sent.set()
            self.server.finish.wait(3)
            try:
                for _ in range(79):
                    self.wfile.write(first)
            except OSError:
                pass
            return
        self.send_header("Transfer-Encoding", "chunked")
        self.send_header("X-Synthetic", "kept")
        self.end_headers()
        self.wfile.write(b"5\r\nfirst\r\n")
        self.wfile.flush()
        self.server.first_sent.set()
        self.server.finish.wait(3)
        self.wfile.write(b"6\r\nsecond\r\n0\r\n\r\n")


class StreamSocket(socket.socket):
    def setsockopt(self, level: int, option: int, value: int) -> None:
        if level != socket.IPPROTO_TCP:
            super().setsockopt(level, option, value)


@contextmanager
def running() -> Iterator[tuple[Server, ThreadingHTTPServer]]:
    with patch.object(ThreadingHTTPServer, "server_bind"), patch.object(ThreadingHTTPServer, "server_activate"):
        upstream = ThreadingHTTPServer(("127.0.0.1", 10001), Upstream)
        proxy = Server(("127.0.0.1", 10002), "http://127.0.0.1:10001", TOKEN, 100)
    upstream.server_port = 10001
    proxy.server_port = 10002
    upstream.daemon_threads = True
    upstream.received = []
    upstream.header_delay = 0.0
    upstream.body_delay = 0.0
    upstream.first_sent = threading.Event()
    upstream.finish = threading.Event()
    peers = []

    def connect(address: tuple[str, int], timeout: float, source_address: object = None) -> socket.socket:
        raw, peer = socket.socketpair()
        client = StreamSocket(fileno=raw.detach())
        client.settimeout(timeout)
        peers.append(peer)
        service = upstream if address[1] == upstream.server_port else proxy
        service.process_request(peer, ("127.0.0.1", 1))
        return client

    try:
        with patch("socket.create_connection", connect):
            yield proxy, upstream
    finally:
        upstream.finish.set()
        for revision in proxy.state.revisions.values():
            for gate in revision.barriers.values():
                gate.set()
        for peer in peers:
            peer.close()
        proxy.server_close()
        upstream.server_close()


def request(proxy: Server, method: str, path: str, body: bytes = b"", token: str | None = None) -> tuple[int, bytes]:
    client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=3)
    headers = {"Authorization": f"Bearer {token}"} if token is not None else {}
    try:
        client.request(method, path, body=body, headers=headers)
        response = client.getresponse()
        return response.status, response.read()
    finally:
        client.close()


def control(proxy: Server, method: str, path: str, body: object) -> tuple[int, object]:
    status, payload = request(proxy, method, path, json.dumps(body).encode(), TOKEN)
    return status, json.loads(payload)


def wait_event(state: State, event: str) -> None:
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        if any(row["event"] == event for row in state.observations(0)["events"]):
            return
        time.sleep(0.001)
    pytest.fail(f"missing event {event}")
