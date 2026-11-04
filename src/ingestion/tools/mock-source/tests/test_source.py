from __future__ import annotations

import asyncio
import copy
import hashlib
import json
from datetime import UTC, datetime
from pathlib import Path
from unittest.mock import AsyncMock, Mock

import pytest
from configuration import STREAMS, Scenario
from server import MAX_BODY_BYTES, MAX_CONNECTIONS, MAX_DATA_REQUESTS, Application, Server
from state import SESSION_TTL_SECONDS, Refusal, State, records

INITIAL = Path(__file__).resolve().parents[5] / "deploy/gitops/environments/local/mock-source/scenarios.json"


def scenario(**controls: object) -> dict[str, object]:
    result = json.loads(INITIAL.read_text())["mock-a"]
    for stream in result["streams"].values():
        stream.update(controls)
    return result


def configured(**controls: object) -> State:
    state = State(100)
    state.configure("mock-a", Scenario.parse(scenario(**controls)))
    return state


@pytest.mark.parametrize("stream", STREAMS)
def test_seed_epoch_and_instance_determine_records(stream: str) -> None:
    spec = scenario()
    parsed = Scenario.parse(spec)
    first = records(parsed, "mock-a", stream, 0, 3, 1)
    assert first == records(parsed, "mock-a", stream, 0, 3, 1)
    assert first[0]["checksum"] == hashlib.sha256(f"7:mock-a:{stream}:0".encode()).hexdigest()
    assert first[0]["last_updated_at"] == spec["epoch"]
    assert first[1]["last_updated_at"] == "2026-01-01T00:00:01Z"
    for field, value in [("seed", 8), ("epoch", "2026-02-01T00:00:00Z")]:
        changed = {**spec, field: value}
        assert first != records(Scenario.parse(changed), "mock-a", stream, 0, 3, 1), field
    assert first != records(parsed, "mock-b", stream, 0, 3, 1)


@pytest.mark.parametrize("stream", STREAMS)
@pytest.mark.parametrize("count", [0, 1, 4, 5])
def test_pagination_returns_every_record_once_and_stops(stream: str, count: int) -> None:
    async def run() -> None:
        state = configured(record_count=count, page_size=2)
        query = {}
        rows = []
        for _ in range(4):
            response = await state.read("mock-a", stream, query, True)
            assert response.status == 200
            rows.extend(response.body["records"])
            token = response.body["next_page_token"]
            if token is None:
                break
            query = {"page_token": token}
        else:
            pytest.fail("pagination did not stop")
        assert [row["id"] for row in rows] == [f"{stream}-{index}" for index in range(count)]

    asyncio.run(run())


@pytest.mark.parametrize(
    "cursor, expected",
    [("2026-01-01T00:00:01Z", [2, 3, 4]), ("2026-01-01T00:00:04Z", []), ("1970-01-01T00:00:00Z", [0, 1, 2, 3, 4])],
)
def test_incremental_cursor_excludes_already_checkpointed_records(cursor: str, expected: list[int]) -> None:
    async def run() -> None:
        state = configured(record_count=5, page_size=10)
        response = await state.read("mock-a", "mock_updates", {"cursor": cursor}, True)
        assert [row["id"] for row in response.body["records"]] == [f"mock_updates-{index}" for index in expected]
        full = await state.read("mock-a", "mock_events", {"cursor": cursor}, True)
        assert len(full.body["records"]) == 5

    asyncio.run(run())


@pytest.mark.parametrize("latency, duration, expected", [(0.01, 0, 0.01), (0, 0.001, 0.03), (0.04, 0.001, 0.04)])
def test_duration_and_latency_are_spread_over_bounded_pages(latency: float, duration: float, expected: float) -> None:
    async def run() -> None:
        state = configured(record_count=4, page_size=2, latency_seconds=latency, duration_minutes=duration)
        start = asyncio.get_running_loop().time()
        first = await state.read("mock-a", "mock_events", {}, True)
        elapsed = asyncio.get_running_loop().time() - start
        assert elapsed >= expected * 0.9
        session = state.sessions[first.body["read_id"]]
        assert session.delay == pytest.approx(expected)
        assert session.delay <= 10

    asyncio.run(run())


@pytest.mark.parametrize("status", [429, 503])
def test_failure_after_completed_pages_keeps_status_and_retry_after(status: int) -> None:
    async def run() -> None:
        state = configured(
            record_count=4, page_size=2, fail_after_pages=1, failure_status=status, retry_after_seconds=3
        )
        first = await state.read("mock-a", "mock_events", {}, True)
        assert first.status == 200
        second = await state.read("mock-a", "mock_events", {"page_token": first.body["next_page_token"]}, True)
        assert second.status == status
        assert second.retry_after == 3

    asyncio.run(run())


@pytest.mark.parametrize("status", [429, 500])
def test_scripted_failures_recover_and_are_isolated_by_stream_and_instance(status: int) -> None:
    async def run() -> None:
        state = configured(record_count=1, fail_requests=2, failure_status=status)
        state.configure("mock-b", Scenario.parse(scenario(record_count=1, fail_requests=2, failure_status=status)))
        for expected in [status, status, 200]:
            assert (await state.read("mock-a", "mock_events", {}, True)).status == expected
        assert (await state.read("mock-a", "mock_updates", {}, True)).status == status
        assert (await state.read("mock-b", "mock_events", {}, True)).status == status

    asyncio.run(run())


@pytest.mark.parametrize("authorized, reject", [(False, False), (True, True)])
def test_auth_rejection_precedes_data_controls(authorized: bool, reject: bool) -> None:
    async def run() -> None:
        state = configured(auth_reject=reject, gate_page=0, fail_requests=1)
        assert (await state.read("mock-a", "mock_events", {}, authorized)).status == 401
        assert not state.instances["mock-a"].failures
        assert not state.instances["mock-a"].gates

    asyncio.run(run())


def test_gated_read_retains_revision_until_explicit_release_and_reset_requires_idle() -> None:
    async def run() -> None:
        state = configured(record_count=3, page_size=2, gate_page=0)
        task = asyncio.create_task(state.read("mock-a", "mock_events", {}, True))
        for _ in range(20):
            await asyncio.sleep(0)
            if any(row.get("event") == "gate_entered" for row in state.journal):
                break
        assert not task.done()
        assert state.observations(0, "mock-a")["active_requests"][0]["revision"] == 1
        with pytest.raises(Refusal) as caught:
            state.reset("mock-a")
        assert caught.value.status == 409
        state.configure("mock-a", Scenario.parse(scenario(record_count=1, page_size=1)))
        assert (await state.read("mock-a", "mock_events", {}, True)).body["revision"] == 2
        state.release("mock-a", 1, "mock_events")
        first = await asyncio.wait_for(task, 1)
        with pytest.raises(Refusal):
            state.reset("mock-a")
        second = await state.read("mock-a", "mock_events", {"page_token": first.body["next_page_token"]}, True)
        assert second.body["revision"] == 1
        assert second.body["records"][0]["id"] == "mock_events-2"
        assert [row["event"] for row in state.journal if str(row["event"]).startswith("gate_")] == [
            "gate_entered",
            "gate_released",
            "gate_passed",
        ]
        baseline = state.sequence
        state.reset("mock-a")
        assert not state.sessions
        assert "mock-a" not in state.instances
        assert state.observations(baseline, None)["events"][0]["event"] == "reset"

    asyncio.run(run())


def test_journal_has_ordered_utc_events_after_baseline_and_bounded_retention() -> None:
    state = State(3)
    for index in range(6):
        state.event(event="synthetic", instance="mock-a" if index % 2 else "mock-b")
    result = state.observations(1, None)
    assert result["truncated"] is True
    assert result["oldest_sequence"] == 4
    assert result["latest_sequence"] == 6
    assert result["generation"] == state.generation
    assert State(3).generation != state.generation
    assert [row["sequence"] for row in result["events"]] == [4, 5, 6]
    assert [row["sequence"] for row in state.observations(4, "mock-a")["events"]] == [6]
    assert state.observations(6, None)["events"] == []
    assert state.observations(4, None)["truncated"] is False
    for row in result["events"]:
        assert datetime.fromisoformat(row["timestamp"]).tzinfo == UTC


def test_probe_traffic_does_not_consume_failures_or_enter_gates() -> None:
    async def run() -> None:
        state = configured(fail_requests=1, gate_page=0)
        app = Application(state, "synthetic-data", "synthetic-control")
        headers = {"authorization": "Bearer synthetic-data"}
        probe = await app.route("GET", "/instances/mock-a/probe", headers, b"")
        assert probe.status == 200 and probe.body == {"records": []}
        assert not state.sessions
        data = await app.route("GET", "/instances/mock-a/streams/mock_events", headers, b"")
        assert data.status == 503
        assert [row["traffic"] for row in state.journal if "traffic" in row] == ["probe", "data", "data"]

    asyncio.run(run())


@pytest.mark.parametrize(
    "change",
    [
        {"page_size": 0},
        {"record_count": -1},
        {"latency_seconds": float("nan")},
        {"latency_seconds": 11},
        {"duration_minutes": 60},
        {"failure_status": 200},
        {"auth_reject": 1},
        {"gate_page": -1},
        {"retry_after_seconds": 61},
    ],
)
def test_invalid_controls_are_rejected_before_installing_revision(change: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        Scenario.parse(scenario(**change))


def test_control_plane_requires_separate_token_and_complete_valid_scenario() -> None:
    async def run() -> None:
        state = State(100)
        app = Application(state, "synthetic-data", "synthetic-control")
        with pytest.raises(Refusal) as caught:
            await app.route("PUT", "/control/instances/mock-a", {"authorization": "Bearer synthetic-data"}, b"{}")
        assert caught.value.status == 401
        headers = {"authorization": "Bearer synthetic-control"}
        response = await app.route("PUT", "/control/instances/mock-a", headers, json.dumps(scenario()).encode())
        assert response.body == {"instance": "mock-a", "revision": 1, "generation": state.generation}
        assert (await app.route("GET", "/health", {}, b"")).status == 200
        observed = await app.route("GET", "/observations?after=0&instance=mock-a", headers, b"")
        assert observed.body["latest_sequence"] == 1
        assert (await app.route("POST", "/control/instances/mock-a/reset", headers, b"")).status == 200
        invalid = copy.deepcopy(scenario())
        del invalid["streams"]["mock_events"]["page_size"]
        with pytest.raises(ValueError):
            await app.route("PUT", "/control/instances/mock-a", headers, json.dumps(invalid).encode())

    asyncio.run(run())


def test_data_concurrency_is_bounded_while_control_release_remains_available() -> None:
    async def run() -> None:
        state = configured(record_count=1, gate_page=0)
        app = Application(state, "synthetic-data", "synthetic-control")
        tasks = [
            asyncio.create_task(
                app.route(
                    "GET", "/instances/mock-a/streams/mock_events", {"authorization": "Bearer synthetic-data"}, b""
                )
            )
            for _ in range(MAX_DATA_REQUESTS)
        ]
        await asyncio.sleep(0)
        response = await app.route("GET", "/instances/mock-a/streams/mock_events", {}, b"")
        assert response.status == 503
        release = await app.route(
            "POST",
            "/control/instances/mock-a/release",
            {"authorization": "Bearer synthetic-control"},
            b'{"revision":1,"stream":"mock_events"}',
        )
        assert release.status == 200
        assert all(response.status == 200 for response in await asyncio.gather(*tasks))
        assert app.data_requests == 0

    asyncio.run(run())


def test_abandoned_read_expires_but_an_active_read_cannot_be_reset() -> None:
    async def run() -> None:
        state = configured(record_count=4, page_size=2)
        first = await state.read("mock-a", "mock_events", {}, True)
        session = state.sessions[first.body["read_id"]]
        session.touched -= SESSION_TTL_SECONDS + 1
        session.active = 1
        with pytest.raises(Refusal):
            state.reset("mock-a")
        session.active = 0
        state.reset("mock-a")
        assert not state.sessions

    asyncio.run(run())


def test_http_transport_bounds_body() -> None:
    async def run() -> None:
        state = configured(record_count=1)
        server = Server(Application(state, "synthetic-secret-data", "synthetic-secret-control"))
        reader = asyncio.StreamReader()
        reader.feed_data(
            (
                f"PUT /control/instances/mock-a HTTP/1.1\r\nHost: example.invalid\r\n"
                f"Content-Length: {MAX_BODY_BYTES + 1}\r\n\r\n"
            ).encode()
        )
        with pytest.raises(Refusal) as caught:
            await server.request(reader)
        assert caught.value.status == 413

    asyncio.run(run())


def test_structured_logs_exclude_authentication_and_bodies(caplog: pytest.LogCaptureFixture) -> None:
    async def run() -> None:
        state = configured(record_count=1)
        app = Application(state, "synthetic-secret-data", "synthetic-secret-control")
        with caplog.at_level("INFO"):
            await app.route(
                "GET",
                "/instances/mock-a/streams/mock_events",
                {"authorization": "Bearer synthetic-secret-data"},
                b"synthetic-secret-body",
            )
        rows = [json.loads(record.message) for record in caplog.records]
        assert rows[-1]["status"] == 200
        assert "synthetic-secret" not in caplog.text

    asyncio.run(run())


@pytest.mark.parametrize(
    "epoch", ["2026-01-01", "2026-01-01T00:00:00+01:00", "2026-01-01T00:00:00.123Z", "9999-12-31T00:00:00Z"]
)
def test_epoch_is_utc_whole_seconds_with_room_for_generated_records(epoch: str) -> None:
    with pytest.raises(ValueError):
        Scenario.parse({**scenario(), "epoch": epoch})


@pytest.mark.parametrize("token", ["missing:2", "missing:invalid"])
def test_unknown_continuations_are_expired_without_creating_a_new_read(token: str) -> None:
    async def run() -> None:
        state = configured()
        with pytest.raises(Refusal) as caught:
            await state.read("mock-a", "mock_events", {"page_token": token}, True)
        assert caught.value.status == 410
        assert not state.sessions

    asyncio.run(run())


def test_continuations_cannot_cross_streams_or_instances() -> None:
    async def run() -> None:
        state = configured(record_count=4, page_size=2)
        state.configure("mock-b", Scenario.parse(scenario(record_count=4, page_size=2)))
        response = await state.read("mock-a", "mock_events", {}, True)
        token = response.body["next_page_token"]
        for instance, stream in [("mock-b", "mock_events"), ("mock-a", "mock_updates")]:
            with pytest.raises(Refusal) as caught:
                await state.read(instance, stream, {"page_token": token}, True)
            assert caught.value.status == 400

    asyncio.run(run())


@pytest.mark.parametrize("status", [429, 499])
def test_http_response_preserves_injected_status_retry_after_and_json_length(status: int) -> None:
    async def run() -> None:
        state = configured(fail_requests=1, failure_status=status, retry_after_seconds=3)
        server = Server(Application(state, "synthetic-data", "synthetic-control"))
        reader = asyncio.StreamReader()
        reader.feed_data(
            b"GET /instances/mock-a/streams/mock_events HTTP/1.1\r\n"
            b"Host: example.invalid\r\nAuthorization: Bearer synthetic-data\r\n\r\n"
        )
        writer = Mock(spec=asyncio.StreamWriter)
        writer.drain = AsyncMock()
        writer.wait_closed = AsyncMock()
        await server.handle(reader, writer)
        wire = writer.write.call_args.args[0]
        headers, body = wire.split(b"\r\n\r\n")
        assert headers.startswith(f"HTTP/1.1 {status} ".encode())
        assert b"Retry-After: 3" in headers
        assert f"Content-Length: {len(body)}".encode() in headers
        assert json.loads(body) == {"error": "injected"}
        assert server.connections == 0
        writer.close.assert_called_once()

    asyncio.run(run())


def test_connection_cap_rejects_before_reading_request() -> None:
    async def run() -> None:
        server = Server(Application(configured(), "synthetic-data", "synthetic-control"))
        server.connections = MAX_CONNECTIONS
        writer = Mock(spec=asyncio.StreamWriter)
        await server.handle(asyncio.StreamReader(), writer)
        writer.close.assert_called_once()
        writer.write.assert_not_called()
        assert server.connections == MAX_CONNECTIONS

    asyncio.run(run())
