from __future__ import annotations

import logging
from unittest.mock import Mock, patch

import pytest
import requests
from source_github_v2.source import SourceGitHubV2

logger = logging.getLogger("test")

CONFIG = {"github_token": "tok", "github_organizations": ["acme"], "insight_tenant_id": "T", "insight_source_id": "S"}


def _response(status_code=200, text="ok"):
    resp = Mock()
    resp.status_code = status_code
    resp.text = text
    return resp


class TestCheckConnection:
    @patch("source_github_v2.source.requests.get")
    def test_ok(self, mock_get):
        mock_get.side_effect = [_response(200), _response(200)]
        ok, reason = SourceGitHubV2().check_connection(logger, CONFIG)
        assert ok is True and reason is None
        urls = [call.args[0] for call in mock_get.call_args_list]
        assert urls[0] == "https://api.github.com/rate_limit"
        assert urls[1] == "https://api.github.com/orgs/acme/repos?per_page=1"
        headers = mock_get.call_args_list[0].kwargs["headers"]
        assert headers["Authorization"] == "Bearer tok"

    @patch("source_github_v2.source.requests.get")
    def test_token_rejected(self, mock_get):
        mock_get.return_value = _response(401, text="bad credentials")
        ok, reason = SourceGitHubV2().check_connection(logger, CONFIG)
        assert ok is False
        assert "Token validation failed (401)" in reason

    @pytest.mark.parametrize(
        "code,fragment",
        [(404, "not found or not accessible"), (403, "lacks permission"), (500, "Failed to access org")],
    )
    @patch("source_github_v2.source.requests.get")
    def test_org_errors_mapped_to_reasons(self, mock_get, code, fragment):
        mock_get.side_effect = [_response(200), _response(code, text="boom")]
        ok, reason = SourceGitHubV2().check_connection(logger, CONFIG)
        assert ok is False
        assert fragment in reason

    @patch("source_github_v2.source.requests.get")
    def test_second_org_checked(self, mock_get):
        mock_get.side_effect = [_response(200), _response(200), _response(404)]
        ok, reason = SourceGitHubV2().check_connection(logger, {**CONFIG, "github_organizations": ["a1", "a2"]})
        assert ok is False
        assert "a2" in reason

    @patch("source_github_v2.source.requests.get")
    def test_no_orgs_only_validates_token(self, mock_get):
        mock_get.return_value = _response(200)
        ok, _ = SourceGitHubV2().check_connection(logger, {"github_token": "tok"})
        assert ok is True
        assert mock_get.call_count == 1

    @patch("source_github_v2.source.requests.get")
    def test_network_exception_reported(self, mock_get):
        mock_get.side_effect = requests.ConnectionError("refused")
        ok, reason = SourceGitHubV2().check_connection(logger, CONFIG)
        assert ok is False
        assert "request failed" in reason


class TestStreams:
    def test_wires_nine_streams_cheap_to_expensive(self):
        streams = SourceGitHubV2().streams(CONFIG)
        names = [s.name for s in streams]
        assert names == [
            "repositories",
            "branches",
            "pull_requests",
            "pull_request_reviews",
            "pull_request_comments",
            "pull_request_review_comments",
            "pull_request_commits",
            "commits",
            "file_changes",
        ]

    def test_parent_wiring(self):
        streams = {s.name: s for s in SourceGitHubV2().streams(CONFIG)}
        assert streams["branches"]._parent is streams["repositories"]
        assert streams["pull_requests"]._parent is streams["repositories"]
        assert streams["pull_request_reviews"]._parent is streams["pull_requests"]
        assert streams["pull_request_comments"]._parent is streams["pull_requests"]
        assert streams["pull_request_review_comments"]._parent is streams["pull_requests"]
        assert streams["pull_request_commits"]._parent is streams["pull_requests"]
        assert streams["commits"]._parent is streams["branches"]
        assert streams["file_changes"]._parent is streams["commits"]

    def test_tenant_identity_propagated(self):
        streams = SourceGitHubV2().streams(CONFIG)
        assert all(s._tenant_id == "T" and s._source_id == "S" for s in streams)

    def test_embedded_page_sizes_reach_pr_query(self):
        config = {**CONFIG, "github_embedded_commits_per_pr": 7}
        streams = {s.name: s for s in SourceGitHubV2().streams(config)}
        assert "commits(first: 7)" in streams["pull_requests"]._query()


class TestSpec:
    def test_spec_loads_and_has_required_fields(self):
        spec = SourceGitHubV2().spec(logger)
        props = spec.connectionSpecification["properties"]
        assert "github_token" in props
        assert "github_organizations" in props


class TestMain:
    @patch("airbyte_cdk.entrypoint.launch")
    def test_main_launches_source_with_argv(self, mock_launch, monkeypatch):
        from source_github_v2.source import main

        monkeypatch.setattr("sys.argv", ["source-github-insight-v2", "spec"])
        main()
        (source, args), _ = mock_launch.call_args
        assert isinstance(source, SourceGitHubV2)
        assert args == ["spec"]
