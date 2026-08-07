"""GitHub directory connector test config builder and shared constants."""

from __future__ import annotations

from connector_tests import ConfigBuilder

GRAPHQL_URL = "https://api.github.com/graphql"
ORG = "acme"

CONNECTOR = "git/github-directory"


class GitHubDirectoryConfigBuilder(ConfigBuilder):
    def __init__(self) -> None:
        super().__init__()
        self._config.update(
            {
                "github_token": "test-token",
                "github_organizations": [ORG],
            }
        )
