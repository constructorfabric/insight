"""Claude Team connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

PROXY_URL = "https://proxy.invalid"
ORG_ID = "org-1"
METRICS_URL = f"{PROXY_URL}/api/claude_code/metrics_aggs/users"


class ClaudeTeamConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update({"claude_org_id": ORG_ID, "proxy_url": PROXY_URL, "proxy_auth_token": "token"})

    def with_start_date(self, start_date: str) -> ClaudeTeamConfigBuilder:
        self._config["start_date"] = start_date
        return self
