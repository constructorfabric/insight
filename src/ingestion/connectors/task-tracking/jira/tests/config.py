"""Jira connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

JIRA_URL = "https://jira.example.com"


class JiraConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "jira_instance_url": JIRA_URL,
                "jira_email": "bot@example.com",
                "jira_api_token": "test-token",
                # Keep the incremental window small and deterministic: with the
                # clock frozen at 2026-07-01 and step P30D this yields a single
                # 30-day slice per partition.
                "jira_start_date": "2026-06-01",
            }
        )
