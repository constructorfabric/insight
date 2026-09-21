"""ChatGPT Team connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

PROXY_URL = "https://proxy.invalid"
ACCOUNT_ID = "acc-1"
ORG_ID = "org-1"
LEADERBOARD_URL = f"{PROXY_URL}/api/wham/analytics/usage-leaderboard"


class ChatGptTeamConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "chatgpt_account_id": ACCOUNT_ID,
                "chatgpt_org_id": ORG_ID,
                "proxy_url": PROXY_URL,
                "proxy_auth_token": "token",
            }
        )

    def with_start_date(self, start_date: str) -> ChatGptTeamConfigBuilder:
        self._config["start_date"] = start_date
        return self
