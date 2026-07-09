"""auth.py — the two-header-set contract the whole ADR-0001 design hinges on.

rest_headers() authenticates to api.github.com; download_headers() must NEVER
carry Authorization since signed URLs on copilot-reports.github.com are
pre-authenticated (see cpt-insightspec-constraint-ghcopilot-no-auth-download).
"""

from __future__ import annotations

from source_github_copilot.auth import download_headers, rest_headers


class TestRestHeaders:
    def test_bearer_auth_present(self):
        headers = rest_headers("secret-tok")
        assert headers["Authorization"] == "Bearer secret-tok"

    def test_github_api_version_pinned(self):
        headers = rest_headers("tok")
        assert headers["Accept"] == "application/vnd.github+json"
        assert headers["X-GitHub-Api-Version"] == "2022-11-28"

    def test_token_interpolated_not_hardcoded(self):
        assert rest_headers("tok-a")["Authorization"] != rest_headers("tok-b")["Authorization"]


class TestDownloadHeaders:
    def test_no_authorization_key_at_all(self):
        """The critical invariant: not empty, not None — the key must be absent."""
        headers = download_headers()
        assert "Authorization" not in headers

    def test_accepts_ndjson(self):
        headers = download_headers()
        assert "application/x-ndjson" in headers["Accept"]

    def test_takes_no_token_argument(self):
        # download_headers() is intentionally token-agnostic — signature has no
        # parameter to accidentally thread a token through.
        import inspect

        assert list(inspect.signature(download_headers).parameters) == []
