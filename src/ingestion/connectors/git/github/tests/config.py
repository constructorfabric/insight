"""github connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

GH_URL = "https://api.github.com"
PROXY_URL = "http://git-cli-proxy.example:8085"


class GithubConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "github_token": "test-gh-token",
                "github_organizations": ["acme"],
                "git_proxy_url": PROXY_URL,
                "git_proxy_token": "test-proxy-token",
                "github_start_date": "2026-06-01",
            }
        )
