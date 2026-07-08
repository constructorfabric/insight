from __future__ import annotations

import logging
from unittest.mock import Mock, patch

import pytest
import requests

from source_bitbucket_cloud.source import SourceBitbucketCloud

logger = logging.getLogger("test")

CONFIG = {
    "bitbucket_token": "tok",
    "bitbucket_workspaces": ["ws"],
    "insight_tenant_id": "T",
    "insight_source_id": "S",
}


def _response(status_code=200, text="ok"):
    resp = Mock()
    resp.status_code = status_code
    resp.text = text
    return resp


class TestCheckConnection:
    def test_empty_workspaces_fails_fast(self):
        ok, reason = SourceBitbucketCloud().check_connection(
            logger, {**CONFIG, "bitbucket_workspaces": []},
        )
        assert ok is False
        assert "bitbucket_workspaces is empty" in reason

    @patch("source_bitbucket_cloud.source.requests.get")
    def test_ok(self, mock_get):
        mock_get.return_value = _response(200)
        ok, reason = SourceBitbucketCloud().check_connection(logger, CONFIG)
        assert ok is True and reason is None
        url = mock_get.call_args[0][0]
        assert url == "https://api.bitbucket.org/2.0/repositories/ws?pagelen=1"
        headers = mock_get.call_args[1]["headers"]
        assert headers["Authorization"] == "Bearer tok"

    @patch("source_bitbucket_cloud.source.requests.get")
    def test_basic_auth_with_username(self, mock_get):
        mock_get.return_value = _response(200)
        ok, _ = SourceBitbucketCloud().check_connection(
            logger, {**CONFIG, "bitbucket_username": "alice"},
        )
        assert ok is True
        headers = mock_get.call_args[1]["headers"]
        assert headers["Authorization"].startswith("Basic ")

    @pytest.mark.parametrize("code,fragment", [
        (401, "Authentication failed"),
        (404, "not found or not accessible"),
        (403, "lacks permission"),
        (500, "Failed to access workspace"),
    ])
    @patch("source_bitbucket_cloud.source.requests.get")
    def test_error_codes_mapped_to_reasons(self, mock_get, code, fragment):
        mock_get.return_value = _response(code, text="boom")
        ok, reason = SourceBitbucketCloud().check_connection(logger, CONFIG)
        assert ok is False
        assert fragment in reason

    @patch("source_bitbucket_cloud.source.requests.get")
    def test_second_workspace_checked(self, mock_get):
        mock_get.side_effect = [_response(200), _response(404)]
        ok, reason = SourceBitbucketCloud().check_connection(
            logger, {**CONFIG, "bitbucket_workspaces": ["ws1", "ws2"]},
        )
        assert ok is False
        assert "ws2" in reason

    @patch("source_bitbucket_cloud.source.requests.get")
    def test_network_exception_reported(self, mock_get):
        mock_get.side_effect = requests.ConnectionError("refused")
        ok, reason = SourceBitbucketCloud().check_connection(logger, CONFIG)
        assert ok is False
        assert "request failed" in reason


class TestStreams:
    def test_wires_seven_streams_cheap_to_expensive(self):
        streams = SourceBitbucketCloud().streams(CONFIG)
        names = [s.name for s in streams]
        assert names == [
            "repositories", "branches", "pull_requests",
            "pull_request_comments", "pull_request_commits",
            "commits", "file_changes",
        ]

    def test_parent_wiring(self):
        streams = {s.name: s for s in SourceBitbucketCloud().streams(CONFIG)}
        assert streams["branches"].parent is streams["repositories"]
        assert streams["pull_requests"].parent is streams["repositories"]
        assert streams["pull_request_comments"].parent is streams["pull_requests"]
        assert streams["pull_request_commits"].parent is streams["pull_requests"]
        assert streams["commits"].parent is streams["branches"]
        assert streams["file_changes"].parent is streams["commits"]

    def test_tenant_identity_propagated(self):
        streams = SourceBitbucketCloud().streams(CONFIG)
        assert all(s._tenant_id == "T" and s._source_id == "S" for s in streams)


class TestSpec:
    def test_spec_loads_and_has_required_fields(self):
        spec = SourceBitbucketCloud().spec(logger)
        props = spec.connectionSpecification["properties"]
        assert "bitbucket_token" in props
        assert "bitbucket_workspaces" in props
