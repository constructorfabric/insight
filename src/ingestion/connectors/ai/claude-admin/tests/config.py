"""Claude Admin connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

API_BASE = "https://api.anthropic.com"


class ClaudeAdminConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        """Seed the base config with a test admin key and a fixed, time-full start_date."""
        super().__init__()
        self._config.update(
            {
                "admin_api_key": "test-admin-key",
                # Full ISO 8601 with time — the messages_usage and cost_report
                # cursors parse strictly and reject bare YYYY-MM-DD. With the clock
                # frozen at 2026-04-27 and step P1D this yields a small, fixed set
                # of one-day slices from 2026-04-24.
                "start_date": "2026-04-24T00:00:00Z",
            }
        )
