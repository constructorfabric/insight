"""SourceGitHubCopilot — spec loading, the 4-branch check_connection() waterfall
(the operator-facing error surface), and streams() wiring.
"""

from __future__ import annotations

from unittest.mock import patch

import requests

from source_github_copilot.source import SourceGitHubCopilot
from tests.conftest import BASE_CONFIG, FakeResponse


def cfg(**overrides):
    return {**BASE_CONFIG, **overrides}


class TestSpec:
    def test_spec_loads_required_fields(self):
        spec = SourceGitHubCopilot().spec(None)
        props = spec.connectionSpecification["properties"]
        for field in ("insight_tenant_id", "insight_source_id", "github_token", "github_org"):
            assert field in props

    def test_github_token_marked_secret(self):
        spec = SourceGitHubCopilot().spec(None)
        assert spec.connectionSpecification["properties"]["github_token"]["airbyte_secret"] is True


class TestCheckConnectionSourceId:
    def test_missing_source_id_fails_fast(self):
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg(insight_source_id=""))
        assert ok is False
        assert "insight_source_id" in reason

    def test_whitespace_only_source_id_fails_fast(self):
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg(insight_source_id="   "))
        assert ok is False
        assert "insight_source_id" in reason

    @patch("source_github_copilot.source.requests.get")
    def test_missing_source_id_never_calls_github(self, mock_get):
        SourceGitHubCopilot().check_connection(None, cfg(insight_source_id=""))
        mock_get.assert_not_called()


class TestCheckConnectionTokenValidity:
    @patch("source_github_copilot.source.requests.get")
    def test_invalid_pat_401(self, mock_get):
        mock_get.return_value = FakeResponse(status_code=401)
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "401" in reason

    @patch("source_github_copilot.source.requests.get")
    def test_rate_limit_probe_unexpected_status(self, mock_get):
        mock_get.return_value = FakeResponse(status_code=500, text="boom")
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "Token validation failed" in reason


class TestCheckConnectionSeatsAccess:
    @patch("source_github_copilot.source.requests.get")
    def test_org_not_found_404(self, mock_get):
        mock_get.side_effect = [FakeResponse(status_code=200), FakeResponse(status_code=404)]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "not found" in reason

    @patch("source_github_copilot.source.requests.get")
    def test_missing_scope_403(self, mock_get):
        mock_get.side_effect = [FakeResponse(status_code=200), FakeResponse(status_code=403)]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "manage_billing:copilot" in reason

    @patch("source_github_copilot.source.requests.get")
    def test_seats_unexpected_status(self, mock_get):
        mock_get.side_effect = [FakeResponse(status_code=200), FakeResponse(status_code=500, text="x")]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "Failed to access seats endpoint" in reason


class TestCheckConnectionMetricsPolicyProbe:
    @patch("source_github_copilot.source.requests.get")
    def test_usage_metrics_policy_disabled_403(self, mock_get):
        mock_get.side_effect = [
            FakeResponse(status_code=200),
            FakeResponse(status_code=200),
            FakeResponse(status_code=403),
        ]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "Copilot usage metrics" in reason

    @patch("source_github_copilot.source.requests.get")
    def test_metrics_endpoint_404(self, mock_get):
        mock_get.side_effect = [
            FakeResponse(status_code=200),
            FakeResponse(status_code=200),
            FakeResponse(status_code=404),
        ]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "404" in reason

    @patch("source_github_copilot.source.requests.get")
    def test_metrics_probe_unexpected_status(self, mock_get):
        mock_get.side_effect = [
            FakeResponse(status_code=200),
            FakeResponse(status_code=200),
            FakeResponse(status_code=400, text="bad"),
        ]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "Failed to probe metrics endpoint" in reason

    @patch("source_github_copilot.source.requests.get")
    def test_metrics_probe_200_succeeds(self, mock_get):
        mock_get.side_effect = [
            FakeResponse(status_code=200),
            FakeResponse(status_code=200),
            FakeResponse(status_code=200),
        ]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert (ok, reason) == (True, None)

    @patch("source_github_copilot.source.requests.get")
    def test_metrics_probe_204_is_a_valid_empty_response(self, mock_get):
        """204 means no data for that day yet — not a failure (PRD §3.1)."""
        mock_get.side_effect = [
            FakeResponse(status_code=200),
            FakeResponse(status_code=200),
            FakeResponse(status_code=204),
        ]
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert (ok, reason) == (True, None)


class TestCheckConnectionNetworkFailure:
    @patch("source_github_copilot.source.requests.get")
    def test_connection_error_surfaced_not_raised(self, mock_get):
        mock_get.side_effect = requests.ConnectionError("dns fail")
        ok, reason = SourceGitHubCopilot().check_connection(None, cfg())
        assert ok is False
        assert "GitHub API request failed" in reason


class TestStreams:
    def test_returns_three_streams_in_order(self):
        streams = SourceGitHubCopilot().streams(cfg())
        assert [s.name for s in streams] == [
            "copilot_seats",
            "copilot_user_metrics",
            "copilot_org_metrics",
        ]

    def test_shared_identity_propagated_to_every_stream(self):
        streams = SourceGitHubCopilot().streams(cfg())
        assert all(
            s._token == "tok" and s._tenant_id == "T" and s._source_id == "S" and s._org == "acme"
            for s in streams
        )

    def test_lookback_days_defaults_to_seven(self):
        streams = SourceGitHubCopilot().streams(cfg())
        user_metrics, org_metrics = streams[1], streams[2]
        assert user_metrics._lookback_days == 7
        assert org_metrics._lookback_days == 7

    def test_lookback_days_custom_value_applied(self):
        streams = SourceGitHubCopilot().streams(cfg(metrics_lookback_days=14))
        assert streams[1]._lookback_days == 14
        assert streams[2]._lookback_days == 14

    def test_lookback_days_invalid_value_falls_back_to_seven(self):
        streams = SourceGitHubCopilot().streams(cfg(metrics_lookback_days="not-a-number"))
        assert streams[1]._lookback_days == 7
        assert streams[2]._lookback_days == 7

    def test_start_date_passed_through_to_metrics_streams(self):
        streams = SourceGitHubCopilot().streams(cfg(github_start_date="2026-01-01"))
        assert streams[1]._start_date == "2026-01-01"
        assert streams[2]._start_date == "2026-01-01"

    def test_seats_stream_has_no_start_date_notion(self):
        """Seats is full-refresh — it must not carry incremental cursor state."""
        streams = SourceGitHubCopilot().streams(cfg())
        assert not hasattr(streams[0], "_start_date")
