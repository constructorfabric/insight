from __future__ import annotations

import json

import pytest
from proxy_state import State


@pytest.mark.parametrize(
    "method,path,endpoint",
    [
        ("GET", "/api/public/v1/jobs?limit=10", "/api/public/v1/jobs"),
        ("POST", "/api/v1/workspaces/list_all?token=synthetic", "/api/v1/workspaces/list_all"),
        ("POST", "/api/v1/source_definitions/list_latest", "/api/v1/source_definitions/list_latest"),
        ("GET", "/api/v1/health-check", "/api/v1/health-check"),
        ("GET", "/api/public/v1/jobs/42", "/api/public/v1/jobs/{redacted}"),
        (
            "GET",
            "/api/public/v1/connections/abcdef00-0000-0000-0000-000000000001",
            "/api/public/v1/connections/{redacted}",
        ),
        ("GET", "/api/v1/connections/abcdef00000000000000000000000001", "/api/v1/connections/{redacted}"),
        ("GET", "/api/v1/connections/user%40example.com", "/api/v1/connections/{redacted}"),
        ("GET", "/api/v1/connections/SyntheticValue", "/api/v1/connections/{redacted}"),
    ],
)
def test_journal_preserves_route_words_and_redacts_values(method: str, path: str, endpoint: str) -> None:
    state = State(10)
    state.begin(method, path, {"connection_id": [], "job_id": []})
    event = state.observations(0)["events"][0]
    assert event["method"] == method, path
    assert event["endpoint"] == endpoint, path
    assert "?" not in json.dumps(event), path
