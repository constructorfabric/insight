"""Zendesk connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

SUBDOMAIN = "example"
BASE_URL = f"https://{SUBDOMAIN}.zendesk.com/api/v2"

# The clock is frozen at 2026-07-01 and start_date is 2026-06-01, so the
# DatetimeBasedCursor (no `step`) yields exactly one slice per stream.
NOW = "2026-07-01T00:00:00Z"
START_DATE = "2026-06-01"
START_EPOCH = "1780272000"  # 2026-06-01T00:00:00Z


class ZendeskConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "zendesk_subdomain": SUBDOMAIN,
                "zendesk_email": "support-bot@example.com",
                "zendesk_api_token": "test-token",
                "start_date": START_DATE,
            }
        )
