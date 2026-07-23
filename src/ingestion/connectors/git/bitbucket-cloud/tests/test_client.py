from __future__ import annotations

from email.utils import formatdate

import pytest

from source_bitbucket_cloud.client import BitbucketApiError, BitbucketClient

BASE = "https://api.bitbucket.org/2.0/"


class Response:
    def __init__(self, status_code=200, body=None, headers=None, url=BASE + "x", text=""):
        self.status_code = status_code
        self._body = body
        self.headers = headers or {}
        self.url = url
        self.text = text

    def json(self):
        if isinstance(self._body, Exception):
            raise self._body
        return self._body


class FakeSession:
    def __init__(self, responses):
        self._responses = list(responses)
        self.headers = {}
        self.calls = []

    def request(self, method, url, params=None, data=None, timeout=None):
        self.calls.append((method, url, params, data))
        nxt = self._responses.pop(0)
        if isinstance(nxt, Exception):
            raise nxt
        return nxt


@pytest.fixture
def no_sleep(monkeypatch):
    monkeypatch.setattr("source_bitbucket_cloud.client.time.sleep", lambda *_a, **_k: None)


def make_client(responses):
    client = BitbucketClient("tok")
    client._session = FakeSession(responses)
    return client


class TestRequestRetries:
    def test_retries_then_succeeds(self, no_sleep):
        client = make_client([Response(503), Response(200, body={"ok": True})])
        response = client.request("GET", "repositories/ws")
        assert response.json() == {"ok": True}
        assert len(client._session.calls) == 2

    def test_transport_error_retried(self, no_sleep):
        import requests

        client = make_client([requests.ConnectionError("boom"), Response(200, body={"ok": True})])
        response = client.request("GET", "repositories/ws")
        assert response.json() == {"ok": True}

    def test_non_retryable_raises_immediately(self, no_sleep):
        client = make_client([Response(400, text="bad request")])
        with pytest.raises(BitbucketApiError) as exc:
            client.request("GET", "repositories/ws")
        assert exc.value.status_code == 400
        assert len(client._session.calls) == 1


class TestRetryDelay:
    def test_retry_after_seconds(self):
        client = make_client([])
        assert client._retry_delay(Response(headers={"Retry-After": "30"}), 0) == 30.0

    def test_retry_after_caps_at_300(self):
        client = make_client([])
        assert client._retry_delay(Response(headers={"Retry-After": "9999"}), 0) == 300.0

    def test_retry_after_http_date(self, monkeypatch):
        now = 1_800_000_000.0
        monkeypatch.setattr("source_bitbucket_cloud.client.time.time", lambda: now)
        client = make_client([])
        header = {"Retry-After": formatdate(now + 60, usegmt=True)}
        assert client._retry_delay(Response(headers=header), 0) == pytest.approx(60.0, abs=1.0)

    def test_ratelimit_reset(self, monkeypatch):
        now = 1_800_000_000.0
        monkeypatch.setattr("source_bitbucket_cloud.client.time.time", lambda: now)
        client = make_client([])
        header = {"X-RateLimit-Reset": str(now + 45)}
        assert client._retry_delay(Response(headers=header), 0) == pytest.approx(45.0, abs=1.0)

    def test_default_exponential_backoff(self):
        client = make_client([])
        assert client._retry_delay(Response(headers={}), 3) == 8.0


class TestPaginate:
    def test_walks_pages_following_next(self):
        client = make_client(
            [
                Response(body={"values": [{"n": 1}], "next": BASE + "repositories/ws?page=2"}),
                Response(body={"values": [{"n": 2}]}),
            ]
        )
        assert [row["n"] for row in client.paginate("repositories/ws")] == [1, 2]

    def test_optional_absent_on_403(self):
        client = make_client([Response(403)])
        present, records = client.paginate_optional("repositories/ws/pipelines")
        assert present is False
        assert list(records) == []

    def test_optional_stops_gracefully_on_later_404(self):
        client = make_client(
            [
                Response(body={"values": [{"n": 1}], "next": BASE + "x?page=2"}),
                Response(404),
            ]
        )
        present, records = client.paginate_optional("repositories/ws/pipelines")
        assert present is True
        assert [row["n"] for row in records] == [1]


class TestFieldMapping:
    def test_repositories_maps_fields_and_skips_forks(self):
        client = make_client(
            [
                Response(
                    body={
                        "values": [
                            {
                                "uuid": "{r-1}",
                                "slug": "repo",
                                "workspace": {"uuid": "{w-1}", "slug": "ws"},
                                "mainbranch": {"name": "main"},
                                "has_issues": True,
                            },
                            {"uuid": "{r-2}", "slug": "fork", "parent": {"uuid": "{r-1}"}},
                        ]
                    }
                )
            ]
        )
        repositories = client.repositories(["ws"], skip_forks=True)
        assert [repo.slug for repo in repositories] == ["repo"]
        repo = repositories[0]
        assert repo.uuid == "{r-1}"
        assert repo.workspace == "ws"
        assert repo.workspace_uuid == "{w-1}"
        assert repo.mainbranch_name == "main"
        assert repo.has_issues is True

    def test_branches_maps_and_marks_default(self):
        from tests.conftest import repository

        client = make_client(
            [
                Response(
                    body={
                        "values": [
                            {"name": "main", "target": {"hash": "a1", "date": "2026-06-01T00:00:00+00:00"}},
                            {"name": "feature", "target": {"hash": "b2"}},
                        ]
                    }
                )
            ]
        )
        branches = client.branches(repository())
        assert [branch.name for branch in branches] == ["main", "feature"]
        assert branches[0].is_default is True
        assert branches[0].head_sha == "a1"
        assert branches[1].is_default is False
