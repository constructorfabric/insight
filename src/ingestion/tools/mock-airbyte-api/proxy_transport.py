from __future__ import annotations

import http.client
import io
import json
import time
from http.server import BaseHTTPRequestHandler
from urllib.parse import urlsplit

from proxy_rules import (
    MAX_BODY_BYTES,
    MAX_REWRITE_BYTES,
    Barrier,
    Delay,
    DelayResponse,
    DropResponse,
    Fault,
    Malformed,
    Rewrite,
    Status,
    Truncated,
    identities,
    integer,
    object_map,
    request_ids,
    rewrite,
)
from proxy_state import Refusal, Revision

CHUNK_BYTES = 65536
IO_TIMEOUT_SECONDS = 10
BARRIER_TIMEOUT_SECONDS = 60
HOP_HEADERS = {
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
}


def json_value(body: bytes) -> object:
    try:
        return json.loads(body)
    except (ValueError, UnicodeError, RecursionError):
        return None


class HeaderReader:
    def __init__(self, stream: io.BufferedReader) -> None:
        self.stream = stream
        self.remaining = 8192

    def readline(self, limit: int) -> bytes:
        line = self.stream.readline(min(limit, self.remaining + 1))
        self.remaining -= len(line)
        if self.remaining < 0:
            raise http.client.LineTooLong("request headers")
        return line


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def setup(self) -> None:
        self.request.settimeout(IO_TIMEOUT_SECONDS)
        super().setup()
        self.started = False

    def parse_request(self) -> bool:
        stream = self.rfile
        self.rfile = HeaderReader(stream)
        try:
            return super().parse_request()
        finally:
            self.rfile = stream

    def log_message(self, format: str, *args: object) -> None:
        return

    def send_error(self, code: int, message: str | None = None, explain: str | None = None) -> None:
        self.respond(code, b'{"error":"invalid request"}')

    def respond(self, status: int, body: bytes, truncated: bool = False) -> None:
        self.started = True
        self.send_response_only(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body) + int(truncated)))
        self.send_header("Connection", "close")
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(body)

    def body(self) -> bytes:
        lengths = self.headers.get_all("Content-Length", [])
        transfers = self.headers.get_all("Transfer-Encoding", [])
        if len(lengths) > 1 or len(transfers) > 1 or (lengths and transfers):
            raise Refusal(400)
        if transfers:
            if transfers != ["chunked"]:
                raise Refusal(400)
            chunks = bytearray()
            wire = bytearray()
            while True:
                line = self.rfile.readline(128)
                if not line.endswith(b"\r\n"):
                    raise Refusal(400)
                wire.extend(line)
                size = int(line.split(b";", 1)[0], 16)
                if size < 0 or len(chunks) + size > MAX_BODY_BYTES:
                    raise Refusal(413)
                if size == 0:
                    trailers = bytearray()
                    while True:
                        trailer = self.rfile.readline(8193)
                        trailers.extend(trailer)
                        if len(trailers) > 8192 or not trailer.endswith(b"\r\n"):
                            raise Refusal(400)
                        if trailer == b"\r\n":
                            break
                    wire.extend(trailers)
                    self.raw_body = bytes(wire)
                    return bytes(chunks)
                chunk = self.rfile.read(size)
                if len(chunk) != size or self.rfile.read(2) != b"\r\n":
                    raise Refusal(400)
                chunks.extend(chunk)
                wire.extend(chunk + b"\r\n")
                if len(wire) > 2 * MAX_BODY_BYTES:
                    raise Refusal(413)
        length = int(lengths[0]) if lengths else 0
        if not 0 <= length <= MAX_BODY_BYTES:
            raise Refusal(413)
        body = self.rfile.read(length)
        if len(body) != length:
            raise Refusal(400)
        return body

    def handle_expect_100(self) -> bool:
        self.send_response_only(100)
        self.end_headers()
        return True

    def dispatch(self) -> None:
        self.close_connection = True
        try:
            if not self.path.startswith("/") or self.path.startswith("//") or urlsplit(self.path).fragment:
                raise Refusal(400)
            body = self.body()
            path = urlsplit(self.path).path
            if path == "/health":
                self.respond(200, json.dumps({"status": "ok", "generation": self.server.state.generation}).encode())
            elif path == "/observations" or path.startswith("/control/"):
                self.control(body)
            elif not self.server.data_slots.acquire(blocking=False):
                ids = request_ids(self.path, json_value(body))
                self.server.state.event(
                    "request_refused", status=503, connection_ids=ids["connection_id"], job_ids=ids["job_id"]
                )
                self.respond(503, b'{"error":"busy"}')
            else:
                try:
                    self.proxy(body)
                finally:
                    self.server.data_slots.release()
        except Refusal as exc:
            if not self.started:
                self.respond(exc.status, b'{"error":"request refused"}')
        except (ValueError, KeyError, TypeError, IndexError, RecursionError):
            if not self.started:
                self.respond(400, b'{"error":"invalid request"}')
        except (OSError, http.client.HTTPException):
            if not self.started:
                self.respond(502, b'{"error":"upstream unavailable"}')

    def control(self, body: bytes) -> None:
        import hmac
        from urllib.parse import parse_qs

        if not hmac.compare_digest(self.headers.get("Authorization", ""), f"Bearer {self.server.control_token}"):
            raise Refusal(401)
        parsed = urlsplit(self.path)
        data = json_value(body)
        state = self.server.state
        if self.command == "GET" and parsed.path == "/observations":
            query = parse_qs(parsed.query, strict_parsing=True)
            if set(query) != {"after"} or len(query["after"]) != 1:
                raise Refusal(400)
            response = state.observations(integer(int(query["after"][0]), 0, 2**63 - 1))
        elif self.command == "PUT" and self.path == "/control/scenario":
            response = {"revision": state.configure(data), "generation": state.generation}
        elif self.command == "POST" and self.path == "/control/release":
            data = object_map(data)
            if set(data) != {"revision", "barrier"} or not isinstance(data["barrier"], str):
                raise Refusal(400)
            state.release(integer(data["revision"], 1, 2**63 - 1), data["barrier"])
            response = {"released": True}
        elif self.command == "POST" and self.path == "/control/reset":
            state.reset()
            response = {"reset": True, "latest_sequence": state.sequence}
        else:
            raise Refusal(404)
        self.respond(200, json.dumps(response).encode())

    def proxy(self, body: bytes) -> None:
        state = self.server.state
        request, revision, rule = state.begin(self.command, self.path, request_ids(self.path, json_value(body)))
        outcome = "failed"
        try:
            action = rule.action if rule else None
            if isinstance(action, (Status, Malformed, Truncated)):
                status = action.status if isinstance(action, Status) else 200
                self.respond(status, action.body, isinstance(action, Truncated))
                state.event("response", request=request, status=status, injected=True)
            else:
                self.before_forward(action, request, revision)
                self.forward(body, action, request)
            outcome = "completed"
        except Refusal as exc:
            state.event("response", request=request, status=exc.status, injected=False)
            raise
        except (OSError, http.client.HTTPException):
            state.event("transport_failure", request=request)
            raise
        finally:
            with state.lock:
                state.event("request_finished", request=request, outcome=outcome)
                del state.active[request]

    def before_forward(self, action: Fault | None, request: int, revision: Revision) -> None:
        if action is None:
            return
        if isinstance(action, Delay):
            time.sleep(action.seconds)
        if isinstance(action, Barrier):
            self.server.state.event(
                "barrier_entered", request=request, revision=revision.number, barrier=action.barrier
            )
            if not revision.barriers[action.barrier].wait(BARRIER_TIMEOUT_SECONDS):
                raise Refusal(504)
            self.server.state.event("barrier_passed", request=request)

    def forward(self, body: bytes, action: Fault | None, request: int) -> None:
        url = self.server.upstream
        connection_type = http.client.HTTPSConnection if url.scheme == "https" else http.client.HTTPConnection
        connection = connection_type(url.hostname, url.port, timeout=IO_TIMEOUT_SECONDS)
        try:
            self.send_upstream(connection, body)
            response = connection.getresponse()
            self.server.state.event("upstream_response", request=request, status=response.status)
            if isinstance(action, (DropResponse, DelayResponse)):
                if isinstance(action, DropResponse):
                    self.drop_response(response, request)
                    return
                self.server.state.event(action.kind, request=request)
                time.sleep(action.seconds)
            if isinstance(action, Rewrite):
                self.rewritten(response, action, request)
                return
            self.relay(response, request)
        finally:
            connection.close()

    def drop_response(self, response: http.client.HTTPResponse, request: int) -> None:
        self.started = True
        captured = b""
        try:
            captured = response.read(CHUNK_BYTES + 1)
        except (OSError, http.client.HTTPException):
            self.server.state.event("drop_capture_unavailable", request=request)
        ids = identities(json_value(captured)) if len(captured) <= CHUNK_BYTES else {"connection_id": [], "job_id": []}
        self.server.state.event(
            "drop_response", request=request, connection_ids=ids["connection_id"], job_ids=ids["job_id"]
        )

    def send_upstream(self, connection: http.client.HTTPConnection, body: bytes) -> None:
        connection.putrequest(self.command, self.path, skip_host=True, skip_accept_encoding=True)
        for key, value in self.headers.items():
            connection.putheader(key, value)
        connection.endheaders()
        if self.headers.get("Transfer-Encoding") == "chunked":
            connection.send(self.raw_body)
        else:
            connection.send(body)

    def response_headers(self, response: http.client.HTTPResponse, rewrite_body: bool) -> None:
        self.started = True
        self.send_response_only(response.status, response.reason)
        excluded = HOP_HEADERS | {part.strip().lower() for part in (response.getheader("Connection") or "").split(",")}
        if rewrite_body:
            excluded |= {"content-length", "content-encoding", "etag", "content-md5", "digest"}
        for key, value in response.getheaders():
            if key.lower() not in excluded:
                self.send_header(key, value)
        self.send_header("Connection", "close")

    def relay(self, response: http.client.HTTPResponse, request: int) -> None:
        self.response_headers(response, False)
        self.end_headers()
        captured = bytearray()
        complete = True
        while chunk := response.read1(CHUNK_BYTES):
            if len(captured) + len(chunk) <= CHUNK_BYTES and complete:
                captured.extend(chunk)
            else:
                complete = False
            self.wfile.write(chunk)
        ids = identities(json_value(bytes(captured))) if complete else {"connection_id": [], "job_id": []}
        self.server.state.event(
            "response",
            request=request,
            status=response.status,
            injected=False,
            connection_ids=ids["connection_id"],
            job_ids=ids["job_id"],
        )

    def rewritten(self, response: http.client.HTTPResponse, action: Rewrite, request: int) -> None:
        if response.getheader("Content-Encoding") not in {None, "identity"}:
            raise Refusal(502)
        body = response.read(MAX_REWRITE_BYTES + 1)
        if len(body) > MAX_REWRITE_BYTES:
            raise Refusal(502)
        try:
            value = rewrite(json.loads(body), action.field, action.value)
            encoded = json.dumps(value, separators=(",", ":")).encode()
        except (ValueError, KeyError, TypeError, IndexError, RecursionError) as exc:
            raise Refusal(502) from exc
        if len(encoded) > MAX_REWRITE_BYTES:
            raise Refusal(502)
        self.response_headers(response, True)
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        if self.command != "HEAD":
            self.wfile.write(encoded)
        ids = identities(value)
        self.server.state.event(
            "response",
            request=request,
            status=response.status,
            injected=True,
            connection_ids=ids["connection_id"],
            job_ids=ids["job_id"],
        )

    def __getattr__(self, name: str) -> object:
        if name.startswith("do_"):
            return self.dispatch
        raise AttributeError(name)
