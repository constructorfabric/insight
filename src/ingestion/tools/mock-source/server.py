from __future__ import annotations

import asyncio
import hmac
import json
import logging
import os
import re
from http import HTTPStatus
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from configuration import STREAMS, Scenario, integer, object_map, timestamp
from state import Refusal, Response, State

MAX_BODY_BYTES = 65536
MAX_HEADER_BYTES = 8192
MAX_CONNECTIONS = 64
MAX_DATA_REQUESTS = 16
INSTANCE = re.compile(r"[a-z][a-z0-9-]{0,62}\Z")


class Application:
    def __init__(self, state: State, api_token: str, control_token: str) -> None:
        if not api_token or not control_token:
            raise ValueError("tokens must be nonempty")
        self.state = state
        self.api_token = api_token
        self.control_token = control_token
        self.data_requests = 0

    async def route(self, method: str, target: str, headers: dict[str, str], body: bytes) -> Response:
        parsed = urlsplit(target)
        path = parsed.path.strip("/").split("/")
        query_lists = parse_qs(parsed.query, strict_parsing=True)
        if any(len(values) != 1 for values in query_lists.values()):
            raise Refusal(400)
        query = {key: values[0] for key, values in query_lists.items()}
        if method == "GET" and parsed.path == "/health":
            return Response(200, {"status": "ok", "generation": self.state.generation})
        if path[0] in {"control", "observations"}:
            if not hmac.compare_digest(headers.get("authorization", ""), f"Bearer {self.control_token}"):
                raise Refusal(401)
            return self.control(method, path, query, body)
        if len(path) < 3 or path[0] != "instances" or not INSTANCE.fullmatch(path[1]):
            raise Refusal(404)
        instance = path[1]
        authorized = hmac.compare_digest(headers.get("authorization", ""), f"Bearer {self.api_token}")
        if method == "GET" and path[2] == "probe" and len(path) == 3:
            revision = self.state.instances.get(instance)
            status = 404 if revision is None else 200
            if not authorized or (revision is not None and revision.scenario.events.auth_reject):
                status = 401
            self.state.event(event="probe", instance=instance, traffic="probe", status=status)
            return Response(status, {"records": []})
        if method != "GET" or len(path) != 4 or path[2] != "streams":
            raise Refusal(404)
        if set(query) - {"cursor", "page_token"}:
            raise Refusal(400)
        if "cursor" in query:
            timestamp(query["cursor"])
        if self.data_requests >= MAX_DATA_REQUESTS:
            self.state.event(event="request_refused", instance=instance, stream=path[3], traffic="data", status=503)
            return Response(503, {"error": "busy"}, 1)
        self.data_requests += 1
        try:
            return await self.state.read(instance, path[3], query, authorized)
        except (Refusal, ValueError, KeyError) as exc:
            status = exc.status if isinstance(exc, Refusal) else 400
            self.state.event(event="request_refused", instance=instance, stream=path[3], traffic="data", status=status)
            raise Refusal(status) from None
        finally:
            self.data_requests -= 1

    def control(self, method: str, path: list[str], query: dict[str, str], body: bytes) -> Response:
        if path == ["observations"] and method == "GET":
            if set(query) - {"after", "instance"}:
                raise Refusal(400)
            after = integer(int(query["after"]), 0, 2**63 - 1)
            return Response(200, self.state.observations(after, query.get("instance")))
        if len(path) not in {3, 4} or path[:2] != ["control", "instances"] or not INSTANCE.fullmatch(path[2]):
            raise Refusal(404)
        instance = path[2]
        if method == "PUT" and len(path) == 3:
            revision = self.state.configure(instance, Scenario.parse(json.loads(body)))
            return Response(200, {"instance": instance, "revision": revision, "generation": self.state.generation})
        if method == "POST" and len(path) == 4 and path[3] == "release":
            data = object_map(json.loads(body))
            revision = integer(data["revision"], 1, 2**63 - 1)
            if data["stream"] not in STREAMS:
                raise Refusal(400)
            self.state.release(instance, revision, data["stream"])
            return Response(200, {"released": True, "revision": revision})
        if method == "POST" and len(path) == 4 and path[3] == "reset":
            self.state.reset(instance)
            return Response(200, {"reset": True, "latest_sequence": self.state.sequence})
        raise Refusal(405)


class Server:
    def __init__(self, app: Application) -> None:
        self.app = app
        self.connections = 0

    async def handle(self, reader: asyncio.StreamReader, writer: asyncio.StreamWriter) -> None:
        if self.connections >= MAX_CONNECTIONS:
            writer.close()
            return
        self.connections += 1
        try:
            try:
                response = await self.request(reader)
            except Refusal as exc:
                response = Response(exc.status, {"error": HTTPStatus(exc.status).phrase})
            except (ValueError, KeyError, UnicodeError, asyncio.IncompleteReadError):
                response = Response(400, {"error": "invalid request"})
            except (TimeoutError, asyncio.LimitOverrunError):
                response = Response(408, {"error": "request limit"})
            encoded = json.dumps(response.body, separators=(",", ":")).encode()
            reason = HTTPStatus(response.status).phrase if response.status in HTTPStatus else "Injected failure"
            headers = [
                f"HTTP/1.1 {response.status} {reason}",
                "Content-Type: application/json",
                f"Content-Length: {len(encoded)}",
                "Connection: close",
            ]
            if response.retry_after is not None:
                headers.append(f"Retry-After: {response.retry_after}")
            writer.write(("\r\n".join(headers) + "\r\n\r\n").encode() + encoded)
            await asyncio.wait_for(writer.drain(), timeout=5)
        except (ConnectionError, TimeoutError):
            pass
        finally:
            self.connections -= 1
            writer.close()
            try:
                await writer.wait_closed()
            except ConnectionError:
                pass

    async def request(self, reader: asyncio.StreamReader) -> Response:
        raw = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
        if len(raw) > MAX_HEADER_BYTES:
            raise Refusal(431)
        lines = raw.decode("ascii").split("\r\n")
        method, target, protocol = lines[0].split(" ")
        if protocol != "HTTP/1.1":
            raise Refusal(400)
        headers: dict[str, str] = {}
        for line in lines[1:-2]:
            key, value = line.split(":", 1)
            name = key.lower()
            if name in headers:
                raise Refusal(400)
            headers[name] = value.strip()
        if "transfer-encoding" in headers:
            raise Refusal(400)
        length = int(headers["content-length"]) if "content-length" in headers else 0
        if not 0 <= length <= MAX_BODY_BYTES:
            raise Refusal(413)
        body = await asyncio.wait_for(reader.readexactly(length), timeout=5)
        return await self.app.route(method, target, headers, body)


async def main() -> None:
    logging.basicConfig(level=logging.INFO, format="%(message)s")
    state = State(integer(int(os.environ["MOCK_JOURNAL_RETENTION"]), 1, 10000))
    initial_text = await asyncio.to_thread(Path(os.environ["MOCK_SCENARIOS_FILE"]).read_text)
    initial = object_map(json.loads(initial_text))
    for instance, scenario in initial.items():
        if not INSTANCE.fullmatch(instance):
            raise ValueError("invalid instance")
        state.configure(instance, Scenario.parse(scenario))
    app = Application(state, os.environ["MOCK_API_TOKEN"], os.environ["MOCK_CONTROL_TOKEN"])
    server = Server(app)
    listener = await asyncio.start_server(
        server.handle, os.environ["MOCK_HOST"], integer(int(os.environ["MOCK_PORT"]), 1, 65535), limit=MAX_HEADER_BYTES
    )
    async with listener:
        await listener.serve_forever()


if __name__ == "__main__":
    asyncio.run(main())
