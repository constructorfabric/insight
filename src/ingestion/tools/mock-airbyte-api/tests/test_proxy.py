from __future__ import annotations

import http.client
import json
import socket
import time
from concurrent.futures import ThreadPoolExecutor
from datetime import UTC, datetime

import pytest
from proxy_client import CONNECTION, control, request, rule, running, wait_event
from proxy_rules import MAX_BODY_BYTES, Action, Rule, request_ids
from proxy_server import MAX_CONNECTIONS, MAX_DATA_REQUESTS
from proxy_state import Refusal, State


def test_forwarding_preserves_method_target_auth_duplicate_headers_and_body() -> None:
    with running() as (proxy, upstream):
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=3)
        client.putrequest("POST", "/api/v1/jobs?x=a%2Fb&x=c", skip_host=True, skip_accept_encoding=True)
        for key, value in [
            ("Host", "synthetic.invalid"),
            ("Authorization", "Bearer synthetic-secret-auth"),
            ("X-Duplicate", "one"),
            ("X-Duplicate", "two"),
            ("Content-Length", "4"),
        ]:
            client.putheader(key, value)
        client.endheaders(b"\x00abc")
        response = client.getresponse()
        assert response.status == 201
        assert response.getheader("X-Synthetic") == "kept"
        assert json.loads(response.read())["job"]["id"] == 42
        assert upstream.received == [
            (
                "POST",
                "/api/v1/jobs?x=a%2Fb&x=c",
                [
                    ("Host", "synthetic.invalid"),
                    ("Authorization", "Bearer synthetic-secret-auth"),
                    ("X-Duplicate", "one"),
                    ("X-Duplicate", "two"),
                    ("Content-Length", "4"),
                ],
                b"\x00abc",
            )
        ]
        client.close()
        wait_event(proxy.state, "response")
        assert any(row.get("job_ids") == ["42"] for row in proxy.state.journal)


def test_response_streaming_delivers_bytes_before_upstream_finishes() -> None:
    with running() as (proxy, upstream):
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=1)
        client.request("GET", "/api/v1/jobs")
        response = client.getresponse()
        assert response.getheader("Transfer-Encoding") is None
        assert response.read(5) == b"first"
        assert upstream.first_sent.is_set() and not upstream.finish.is_set()
        upstream.finish.set()
        assert response.read() == b"second"
        client.close()


@pytest.mark.parametrize("status", [200, 409, 500, 502, 503])
def test_status_fault_never_submits_upstream(status: int) -> None:
    with running() as (proxy, upstream):
        proxy.state.configure({"rules": [rule({"type": "status", "status": status, "body": '{"synthetic":true}'})]})
        assert request(proxy, "POST", "/api/v1/jobs") == (status, b'{"synthetic":true}')
        assert upstream.received == []


@pytest.mark.parametrize("kind", ["malformed", "truncated"])
def test_invalid_payload_faults_have_distinct_wire_framing(kind: str) -> None:
    with running() as (proxy, upstream):
        proxy.state.configure({"rules": [rule({"type": kind, "body": "{invalid"})]})
        if kind == "truncated":
            with pytest.raises(http.client.IncompleteRead):
                request(proxy, "POST", "/api/v1/jobs")
        else:
            assert request(proxy, "POST", "/api/v1/jobs") == (200, b"{invalid")
        assert not upstream.received


@pytest.mark.parametrize("kind", ["delay", "delay_response"])
def test_delays_apply_at_the_selected_side_of_upstream_acceptance(kind: str) -> None:
    with running() as (proxy, upstream), ThreadPoolExecutor() as pool:
        proxy.state.configure({"rules": [rule({"type": kind, "seconds": 0.15})]})
        start = time.monotonic()
        pending = pool.submit(request, proxy, "POST", "/api/v1/jobs")
        if kind == "delay_response":
            wait_event(proxy.state, "delay_response")
            assert len(upstream.received) == 1
            assert not pending.done()
        else:
            wait_event(proxy.state, "request_started")
            assert not upstream.received
        assert pending.result()[0] == 201
        assert time.monotonic() - start >= 0.14


def test_accepted_but_lost_submits_once_and_returns_no_response() -> None:
    with running() as (proxy, upstream):
        proxy.state.configure({"rules": [rule({"type": "drop_response"})]})
        with pytest.raises(http.client.RemoteDisconnected):
            request(proxy, "POST", "/api/v1/jobs")
        assert len(upstream.received) == 1
        assert any(row["event"] == "upstream_response" and row["status"] == 201 for row in proxy.state.journal)
        assert any(row["event"] == "drop_response" and row["job_ids"] == ["42"] for row in proxy.state.journal)


@pytest.mark.parametrize(
    "field,value", [(["job", "status"], "cancelled"), (["job", "streamStats"], []), (["job", "streamStats", "0"], 7)]
)
def test_response_rewrite_changes_only_the_selected_field(field: list[str], value: object) -> None:
    with running() as (proxy, _):
        proxy.state.configure({"rules": [rule({"type": "rewrite", "field": field, "value": value})]})
        status, body = request(proxy, "POST", "/api/v1/jobs")
        expected = {"job": {"id": 42, "status": "succeeded", "streamStats": [1]}}
        if len(field) == 3:
            expected["job"]["streamStats"][0] = value
        else:
            expected["job"][field[-1]] = value
        assert status == 201 and json.loads(body) == expected


def test_rewrites_fail_closed_for_missing_fields_and_oversize_responses() -> None:
    with running() as (proxy, _):
        proxy.state.configure({"rules": [rule({"type": "rewrite", "field": ["missing"], "value": []})]})
        assert request(proxy, "POST", "/api/v1/jobs")[0] == 502
        proxy.state.configure({"rules": [rule({"type": "rewrite", "field": ["job", "status"], "value": "cancelled"})]})
        assert request(proxy, "GET", "/api/v1/oversize")[0] == 502


def test_call_numbers_count_only_method_path_and_id_candidates() -> None:
    with running() as (proxy, _):
        proxy.state.configure(
            {
                "rules": [
                    rule(
                        {"type": "status", "status": 409},
                        method="POST",
                        path="/api/v1/connections/*",
                        connection_id=CONNECTION,
                        job_id="42",
                        call=2,
                    )
                ]
            }
        )
        path = f"/api/v1/connections/{CONNECTION}"
        assert request(proxy, "POST", path, b'{"jobId":41}')[0] == 201
        for expected in [201, 409, 201]:
            assert request(proxy, "POST", path, b'{"jobId":42}')[0] == expected
        matcher = Rule.parse(rule({"type": "status", "status": 409}, method="POST", path="/api/v1/jobs/*"))
        assert not matcher.matches("GET", "/api/v1/jobs/42", {})
        assert not matcher.matches("POST", "/api/v1/other/42", {})
        assert request_ids("/api/public/v1/jobs/42", {"connectionId": CONNECTION}) == {
            "connection_id": [CONNECTION],
            "job_id": ["42"],
        }
        assert request_ids("/", {"job": {"id": 42}, "connection_id": CONNECTION})["job_id"] == ["42"]


def test_barriers_keep_captured_revision_and_reset_requires_idle() -> None:
    with running() as (proxy, upstream), ThreadPoolExecutor() as pool:
        assert (
            control(proxy, "PUT", "/control/scenario", {"rules": [rule({"type": "barrier", "barrier": "submit"})]})[1][
                "revision"
            ]
            == 1
        )
        pending = pool.submit(request, proxy, "POST", "/api/v1/jobs")
        wait_event(proxy.state, "barrier_entered")
        assert not pending.done() and not upstream.received
        assert control(proxy, "POST", "/control/reset", {})[0] == 409
        assert control(proxy, "PUT", "/control/scenario", {"rules": []})[1]["revision"] == 2
        assert control(proxy, "POST", "/control/release", {"revision": 2, "barrier": "submit"})[0] == 404
        assert control(proxy, "POST", "/control/release", {"revision": 1, "barrier": "submit"})[0] == 200
        assert pending.result()[0] == 201
        wait_event(proxy.state, "request_finished")
        baseline = proxy.state.sequence
        assert control(proxy, "POST", "/control/reset", {})[0] == 200
        observed = control(proxy, "GET", f"/observations?after={baseline}", {})[1]
        assert [row["event"] for row in observed["events"]] == ["reset"]
        assert control(proxy, "PUT", "/control/scenario", {"rules": []})[1]["revision"] == 3


def test_release_before_entry_and_barrier_timeout_are_bounded(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("proxy_transport.BARRIER_TIMEOUT_SECONDS", 0.01)
    with running() as (proxy, upstream):
        proxy.state.configure({"rules": [rule({"type": "barrier", "barrier": "submit"})]})
        assert request(proxy, "POST", "/api/v1/jobs")[0] == 504
        assert not upstream.received
        proxy.state.release(1, "submit")
        assert request(proxy, "POST", "/api/v1/jobs")[0] == 201


def test_controls_require_auth_and_reject_invalid_inputs_atomically() -> None:
    with running() as (proxy, _):
        assert request(proxy, "GET", "/health")[0] == 200
        for path in ["/control/scenario", "/observations?after=0", "/control/reset"]:
            assert request(proxy, "PUT", path, b"{}", "synthetic-wrong")[0] == 401
        for data in [{}, {"rules": [rule({"type": "delay", "seconds": -1})]}, {"rules": [rule({"type": "unknown"})]}]:
            assert control(proxy, "PUT", "/control/scenario", data)[0] == 400
        assert proxy.state.revision == 0
        assert control(proxy, "GET", "/observations?after=0&after=1", {})[0] == 400
        assert control(proxy, "GET", "/observations?after=-1", {})[0] == 400


def test_journal_bounds_sequences_utc_generation_and_exclusive_baselines() -> None:
    state = State(3)
    for _ in range(6):
        state.event("synthetic")
    result = state.observations(1)
    assert result["truncated"] and result["oldest_sequence"] == 4 and result["latest_sequence"] == 6
    assert [row["sequence"] for row in state.observations(4)["events"]] == [5, 6]
    assert not state.observations(6)["events"] and not state.observations(4)["truncated"]
    assert State(3).generation != state.generation == result["generation"]
    for row in result["events"]:
        assert datetime.fromisoformat(row["timestamp"]).tzinfo == UTC


def test_journal_and_logs_exclude_headers_query_paths_and_bodies(caplog: pytest.LogCaptureFixture) -> None:
    with caplog.at_level("INFO"), running() as (proxy, _):
        assert (
            request(
                proxy,
                "POST",
                "/api/v1/synthetic-secret-Path?token=synthetic-secret-query",
                b'{"password":"synthetic-secret-body"}',
                "synthetic-secret-auth",
            )[0]
            == 201
        )
        wait_event(proxy.state, "request_finished")
        journal = json.dumps(proxy.state.observations(0))
        assert "synthetic-secret" not in journal + caplog.text
        assert any(
            row.get("method") == "POST" and row.get("endpoint") == "/api/v1/{redacted}" for row in proxy.state.journal
        )


def test_data_saturation_keeps_control_available_and_all_limits_are_released() -> None:
    with running() as (proxy, _), ThreadPoolExecutor(max_workers=MAX_DATA_REQUESTS) as pool:
        proxy.state.configure({"rules": [rule({"type": "barrier", "barrier": "submit"})]})
        pending = [pool.submit(request, proxy, "POST", "/api/v1/jobs") for _ in range(MAX_DATA_REQUESTS)]
        deadline = time.monotonic() + 2
        while len(proxy.state.active) != MAX_DATA_REQUESTS and time.monotonic() < deadline:
            time.sleep(0.001)
        assert len(proxy.state.active) == MAX_DATA_REQUESTS
        assert request(proxy, "POST", "/api/v1/jobs")[0] == 503
        assert any(row["event"] == "request_refused" and row["status"] == 503 for row in proxy.state.journal)
        assert control(proxy, "POST", "/control/release", {"revision": 1, "barrier": "submit"})[0] == 200
        assert all(task.result()[0] == 201 for task in pending)
        assert request(proxy, "POST", "/api/v1/jobs")[0] == 201
        assert MAX_CONNECTIONS > MAX_DATA_REQUESTS


@pytest.mark.parametrize(
    "raw",
    [
        b"Content-Length: 1\r\nContent-Length: 1",
        b"Content-Length: 1\r\nTransfer-Encoding: chunked",
        b"Transfer-Encoding: gzip",
        f"Content-Length: {MAX_BODY_BYTES + 1}".encode(),
    ],
)
def test_ambiguous_or_unbounded_request_framing_is_rejected(raw: bytes) -> None:
    with (
        running() as (proxy, upstream),
        socket.create_connection(("127.0.0.1", proxy.server_port), timeout=2) as client,
    ):
        client.sendall(b"POST /api/v1/jobs HTTP/1.1\r\nHost: synthetic.invalid\r\n" + raw + b"\r\n\r\nx")
        expected = 413 if raw == f"Content-Length: {MAX_BODY_BYTES + 1}".encode() else 400
        assert client.recv(1024).startswith(f"HTTP/1.1 {expected}".encode())
        assert not upstream.received


@pytest.mark.parametrize(
    "change",
    [
        {"call": 0},
        {"connection_id": "secret"},
        {"method": "get"},
        {"path": "http://synthetic.invalid"},
        {"name": "bad name"},
        {"unknown": True},
        {"action": {"type": "rewrite", "field": [], "value": 1}},
        {"action": {"type": "delay", "seconds": float("nan")}},
    ],
)
def test_rules_reject_invalid_matchers_and_actions(change: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        Rule.parse({**rule({"type": "status", "status": 503}), **change})


def test_revision_retention_is_bounded_until_idle_reset() -> None:
    state = State(10)
    for _ in range(63):
        state.configure({"rules": []})
    with pytest.raises(Refusal) as caught:
        state.configure({"rules": []})
    assert caught.value.status == 503
    state.reset()
    assert state.configure({"rules": []}) == 64
    assert Action.parse({"type": "status", "status": 409}).body == b""


def test_chunked_request_preserves_extensions_trailers_and_decoded_body() -> None:
    with running() as (proxy, upstream):
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=2)
        client.putrequest("POST", "/api/v1/jobs")
        client.putheader("Transfer-Encoding", "chunked")
        client.putheader("Trailer", "X-Synthetic")
        client.endheaders()
        wire = b"2;synthetic=yes\r\nab\r\n2\r\ncd\r\n0\r\nX-Synthetic: kept\r\n\r\n"
        client.send(wire)
        response = client.getresponse()
        assert response.status == 201
        response.read()
        client.close()
        assert upstream.received[0][-1] == b"abcd"
        assert upstream.chunked_wire == wire
        assert ("Transfer-Encoding", "chunked") in upstream.received[0][2]


def test_connection_limit_closes_excess_connections_before_dispatch() -> None:
    with running() as (proxy, upstream):
        for _ in range(MAX_CONNECTIONS):
            assert proxy.slots.acquire(blocking=False)
        try:
            with socket.create_connection(("127.0.0.1", proxy.server_port), timeout=2) as client:
                assert client.recv(1) == b""
            assert not proxy.state.active and not upstream.received
        finally:
            for _ in range(MAX_CONNECTIONS):
                proxy.slots.release()
        assert request(proxy, "POST", "/api/v1/jobs")[0] == 201


def test_journal_payload_size_is_bounded_without_limiting_id_matching() -> None:
    ids = request_ids("/", [{"jobId": index} for index in range(50)])
    assert len(ids["job_id"]) == 50
    state = State(10)
    _, _, _ = state.begin("POST", "/" + "/".join(["unknown"] * 1000), ids)
    event = state.observations(0)["events"][0]
    assert len(event["endpoint"]) <= 512
    assert len(event["job_ids"]) == 16
    assert len(state.observations(0)["active_requests"][0]["job_ids"]) == 16
    state.event("response", job_ids=ids["job_id"])
    assert len(state.observations(0)["events"][-1]["job_ids"]) == 16


@pytest.mark.parametrize(
    "action",
    [
        {"type": "malformed", "body": None},
        {"type": "barrier", "barrier": None},
        {"type": "delay", "seconds": None},
        {"type": "rewrite", "field": None, "value": 1},
    ],
)
def test_required_action_payloads_cannot_be_null(action: dict[str, object]) -> None:
    with pytest.raises(ValueError):
        Action.parse(action)


def test_large_responses_stream_past_the_rewrite_buffer_limit() -> None:
    with running() as (proxy, upstream):
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=1)
        client.request("GET", "/api/v1/large")
        response = client.getresponse()
        assert response.read(65536) == b"a" * 65536
        assert not upstream.finish.is_set()
        upstream.finish.set()
        chunks = 1
        while chunk := response.read(65536):
            assert chunk == b"a" * 65536
            chunks += 1
        assert chunks == 80
        client.close()


def test_header_limit_rejects_an_unfinished_oversize_header_without_waiting_for_body() -> None:
    with (
        running() as (proxy, upstream),
        socket.create_connection(("127.0.0.1", proxy.server_port), timeout=1) as client,
    ):
        client.sendall(b"POST /api/v1/jobs HTTP/1.1\r\nX-Synthetic: " + b"a" * 8193)
        assert client.recv(1024).startswith(b"HTTP/1.1 431")
        assert not upstream.received


def test_accepted_but_lost_still_drops_when_upstream_body_evidence_times_out(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("proxy_transport.IO_TIMEOUT_SECONDS", 0.02)
    with running() as (proxy, upstream):
        proxy.upstream_timeout = 0.02
        proxy.state.configure({"rules": [rule({"type": "drop_response"})]})
        with pytest.raises(http.client.RemoteDisconnected):
            request(proxy, "POST", "/api/v1/jobs/slow")
        assert len(upstream.received) == 1
        assert any(row["event"] == "upstream_response" and row["status"] == 201 for row in proxy.state.journal)
        assert any(row["event"] == "drop_capture_unavailable" for row in proxy.state.journal)
        assert any(row["event"] == "drop_response" for row in proxy.state.journal)
