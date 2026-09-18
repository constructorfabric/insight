"""gitlab connector test config builder."""

from __future__ import annotations

from connector_tests import ConfigBuilder

GITLAB_URL = "https://gitlab.example.com"
API_URL = f"{GITLAB_URL}/api/v4"
GRAPHQL_URL = f"{GITLAB_URL}/api/graphql"
PROXY_URL = "http://git-cli-proxy.example:8085"


class GitlabConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "gitlab_url": GITLAB_URL,
                "gitlab_token": "test-gitlab-token",
                "gitlab_groups": ["acme"],
                "git_proxy_url": PROXY_URL,
                "git_proxy_token": "test-proxy-token",
                "gitlab_start_date": "2026-06-01",
            }
        )
