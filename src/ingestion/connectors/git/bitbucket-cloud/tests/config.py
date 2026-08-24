"""bitbucket-cloud connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

BB_URL = "https://api.bitbucket.org/2.0"
PROXY_URL = "http://git-cli-proxy.example:8085"


class BitbucketCloudConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "bitbucket_username": "bot@example.com",
                "bitbucket_token": "test-bb-token",
                "bitbucket_workspaces": ["acme"],
                "git_proxy_url": PROXY_URL,
                "git_proxy_token": "test-proxy-token",
                "bitbucket_start_date": "2026-06-01",
            }
        )
