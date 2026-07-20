from __future__ import annotations

from source_github_v2.auth import auth_headers, graphql_headers, rest_headers


class TestAuthHeaders:
    def test_bearer_and_user_agent(self):
        headers = auth_headers("tok")
        assert headers["Authorization"] == "Bearer tok"
        assert headers["User-Agent"].startswith("insight-github-connector/")

    def test_rest_headers_add_api_version(self):
        headers = rest_headers("tok")
        assert headers["Accept"] == "application/vnd.github+json"
        assert headers["X-GitHub-Api-Version"] == "2022-11-28"
        assert headers["Authorization"] == "Bearer tok"

    def test_graphql_headers_are_plain_auth(self):
        assert graphql_headers("tok") == auth_headers("tok")
