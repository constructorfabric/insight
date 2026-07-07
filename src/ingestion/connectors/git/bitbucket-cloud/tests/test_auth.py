from __future__ import annotations

import base64

from source_bitbucket_cloud.auth import auth_headers


def test_bearer_when_no_username():
    headers = auth_headers("tok123")
    assert headers["Authorization"] == "Bearer tok123"
    assert headers["Accept"] == "application/json"
    assert "User-Agent" in headers


def test_basic_when_username_given():
    headers = auth_headers("tok123", username="alice")
    scheme, _, encoded = headers["Authorization"].partition(" ")
    assert scheme == "Basic"
    assert base64.b64decode(encoded).decode() == "alice:tok123"
