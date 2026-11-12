from __future__ import annotations

import asyncio
import json
from pathlib import Path
from unittest.mock import AsyncMock, Mock

import pytest
from configuration import Scenario
from server import Application, Server
from state import State

INITIAL = Path(__file__).resolve().parents[5] / "deploy/gitops/environments/local/mock-source/scenarios.json"


@pytest.mark.parametrize("field,maximum", [("latency_seconds", 10), ("duration_minutes", 60)])
@pytest.mark.parametrize(
    "value",
    [10**400, -(10**400), -1, 61, float("inf"), float("nan"), True, "1"],
    ids=["huge", "negative-huge", "negative", "above-limit", "infinite", "nan", "boolean", "string"],
)
def test_invalid_timing_returns_a_field_error_without_changing_the_scenario(
    field: str, maximum: int, value: object
) -> None:
    async def run() -> None:
        scenario = json.loads(INITIAL.read_text())["mock-a"]
        state = State(100)
        state.configure("mock-a", Scenario.parse(scenario))
        before = state.observations(0, None)
        scenario["streams"]["mock_events"][field] = value
        body = json.dumps(scenario).encode()
        reader = asyncio.StreamReader()
        request_headers = (
            "PUT /control/instances/mock-a HTTP/1.1\r\n"
            "Authorization: Bearer synthetic-control\r\n"
            f"Content-Length: {len(body)}\r\n\r\n"
        ).encode()
        reader.feed_data(request_headers + body)
        writer = Mock(spec=asyncio.StreamWriter)
        writer.drain = AsyncMock()
        writer.wait_closed = AsyncMock()
        server = Server(Application(state, "synthetic-data", "synthetic-control"))
        await server.handle(reader, writer)
        headers, response = writer.write.call_args.args[0].split(b"\r\n\r\n")
        assert headers.startswith(b"HTTP/1.1 400 "), field
        assert json.loads(response) == {"error": f"{field} must be a number between 0 and {maximum}"}
        assert state.observations(0, None) == before, field
        assert state.instances["mock-a"].number == 1, field
        assert server.connections == 0

    asyncio.run(run())
