from __future__ import annotations

import http.client
import json
from http.server import ThreadingHTTPServer

import pytest
from proxy_client import running
from proxy_server import main


@pytest.mark.parametrize("configured,expected", [(None, 300), ("450", 450)])
def test_runtime_upstream_timeout_uses_the_default_or_environment(
    monkeypatch: pytest.MonkeyPatch, configured: str | None, expected: float
) -> None:
    for name, value in {
        "MOCK_HOST": "127.0.0.1",
        "MOCK_PORT": "8080",
        "MOCK_AIRBYTE_UPSTREAM": "http://synthetic.invalid:8001",
        "MOCK_CONTROL_TOKEN": "synthetic-control-token",
        "MOCK_JOURNAL_RETENTION": "100",
    }.items():
        monkeypatch.setenv(name, value)
    monkeypatch.delenv("MOCK_UPSTREAM_TIMEOUT", raising=False)
    if configured is not None:
        monkeypatch.setenv("MOCK_UPSTREAM_TIMEOUT", configured)

    observed = []
    monkeypatch.setattr(ThreadingHTTPServer, "server_bind", lambda server: None)
    monkeypatch.setattr(ThreadingHTTPServer, "server_activate", lambda server: None)
    monkeypatch.setattr(ThreadingHTTPServer, "serve_forever", lambda server: observed.append(server.upstream_timeout))
    main()
    assert observed == [expected], configured


@pytest.mark.parametrize("phase", ["header", "body"])
def test_upstream_responses_slower_than_ten_seconds_are_relayed(phase: str) -> None:
    with running() as (proxy, upstream):
        assert proxy.upstream_timeout == 300
        proxy.upstream_timeout = 15
        setattr(upstream, f"{phase}_delay", 10.2)
        client = http.client.HTTPConnection("127.0.0.1", proxy.server_port, timeout=15)
        try:
            client.request("POST", "/api/v1/sources/discover_schema", body=b"{}")
            response = client.getresponse()
            assert response.status == 201, phase
            assert json.loads(response.read())["job"]["id"] == 42, phase
        finally:
            client.close()
