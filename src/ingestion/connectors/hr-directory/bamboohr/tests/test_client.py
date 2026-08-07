from __future__ import annotations

import time
from email.utils import formatdate

import pytest
import requests
from source_bamboohr.client import BambooClient, BambooHrApiError, BambooHrAuthError, BambooHrDomainError, api_base_url

BASE = "https://acme.bamboohr.com/api/v1/"


class Response:
    def __init__(self, status_code=200, body=None, headers=None, text=""):
        self.status_code = status_code
        self._body = body
        self.headers = headers or {}
        self.url = BASE + "meta/fields"
        self.text = text

    def json(self):
        if isinstance(self._body, Exception):
            raise self._body
        return self._body


class FakeSession:
    def __init__(self, responses):
        self._responses = list(responses)
        self.auth = None
        self.headers = {}
        self.calls = []

    def request(self, method, url, params=None, json=None, timeout=None):
        self.calls.append({"method": method, "url": url, "params": params, "json": json, "timeout": timeout})
        nxt = self._responses.pop(0)
        if isinstance(nxt, Exception):
            raise nxt
        return nxt


def make_client(responses):
    client = BambooClient(domain="acme", api_key="key")
    client._session = FakeSession(responses)
    return client


@pytest.fixture
def slept(monkeypatch):
    delays: list[float] = []
    monkeypatch.setattr("source_bamboohr.client.time.sleep", delays.append)
    monkeypatch.setattr("source_bamboohr.client.random.random", lambda: 0.0)
    return delays


class TestDomain:
    @pytest.mark.parametrize("domain", ["acme", "acme-corp", "a", "acme1", "ACME"])
    def test_a_bare_subdomain_is_accepted(self, domain):
        assert api_base_url(domain) == f"https://{domain}.bamboohr.com/api/v1/"

    @pytest.mark.parametrize(
        "domain",
        [
            "evil.example.com",
            "acme.bamboohr.com",
            "evil.example.com/path",
            "acme/../evil.example.com",
            "acme:8080",
            "user:pass@evil.example.com",
            "acme?x=1",
            "acme#frag",
            "acme corp",
            "-acme",
            "acme-",
            "",
            "   ",
        ],
    )
    def test_anything_that_could_move_the_host_is_rejected(self, domain):
        with pytest.raises(BambooHrDomainError):
            api_base_url(domain)

    @pytest.mark.parametrize("domain", ["evil.example.com", "user:pass@evil.example.com"])
    def test_the_credentials_are_never_attached_to_a_foreign_host(self, domain):
        with pytest.raises(BambooHrDomainError):
            BambooClient(domain=domain, api_key="key")


class TestRequests:
    def test_the_domain_becomes_the_api_host(self, slept):
        client = make_client([Response(200, body=[])])
        client.get("meta/fields")
        assert client._session.calls[0]["url"] == BASE + "meta/fields"

    def test_every_request_is_bounded_by_a_timeout(self, slept):
        client = make_client([Response(200, body=[])])
        client.get("meta/fields")
        assert client._session.calls[0]["timeout"] == (10, 120)

    def test_a_response_that_is_not_json_is_an_error(self, slept):
        client = make_client([Response(200, body=ValueError("boom"))])
        with pytest.raises(RuntimeError, match="invalid JSON"):
            client.get("meta/fields")


class TestRetries:
    @pytest.mark.parametrize("status", [408, 429, 500, 502, 503, 504])
    def test_a_transient_status_is_retried(self, status, slept):
        client = make_client([Response(status), Response(200, body=[])])
        assert client.get("meta/fields") == []
        assert len(client._session.calls) == 2

    def test_a_transport_failure_is_retried(self, slept):
        client = make_client([requests.ConnectionError("reset"), Response(200, body=[])])
        assert client.get("meta/fields") == []

    def test_retries_are_given_up_and_the_last_status_raised(self, slept):
        client = make_client([Response(503, text="busy")] * 6)
        with pytest.raises(BambooHrApiError) as caught:
            client.get("meta/fields")

        assert caught.value.status_code == 503
        assert len(client._session.calls) == 6

    def test_the_wait_grows_between_attempts(self, slept):
        client = make_client([Response(503), Response(503), Response(200, body=[])])
        client.get("meta/fields")
        assert slept == [1.0, 2.0]


class TestRetryAfter:
    def test_a_delay_in_seconds_is_honoured(self, slept):
        client = make_client([Response(429, headers={"Retry-After": "17"}), Response(200, body=[])])
        client.get("meta/fields")
        assert slept == [17.0]

    def test_an_http_date_delay_is_honoured(self, slept):
        retry_at = formatdate(time.time() + 30, usegmt=True)
        client = make_client([Response(503, headers={"Retry-After": retry_at}), Response(200, body=[])])
        client.get("meta/fields")
        assert 25.0 <= slept[0] <= 31.0, f"should wait until {retry_at}"

    def test_a_delay_already_in_the_past_does_not_wait(self, slept):
        retry_at = formatdate(time.time() - 60, usegmt=True)
        client = make_client([Response(503, headers={"Retry-After": retry_at}), Response(200, body=[])])
        client.get("meta/fields")
        assert slept == [0.0]

    def test_an_unparseable_delay_falls_back_to_backoff(self, slept):
        client = make_client([Response(503, headers={"Retry-After": "soon"}), Response(200, body=[])])
        client.get("meta/fields")
        assert slept == [1.0]

    def test_an_absurd_delay_is_capped(self, slept):
        client = make_client([Response(503, headers={"Retry-After": "99999"}), Response(200, body=[])])
        client.get("meta/fields")
        assert slept == [300.0]


class TestRejections:
    @pytest.mark.parametrize("status", [401, 403])
    def test_a_rejected_key_fails_immediately(self, status, slept):
        client = make_client([Response(status, text="denied")])
        with pytest.raises(BambooHrAuthError):
            client.get("meta/fields")

        assert len(client._session.calls) == 1

    @pytest.mark.parametrize(
        ("status", "expected"),
        [(401, "invalid, expired, or revoked"), (403, "lacks permission")],
    )
    def test_the_failure_names_what_to_fix(self, status, expected, slept):
        client = make_client([Response(status)])
        with pytest.raises(BambooHrAuthError, match=expected):
            client.get("meta/fields")

    def test_a_client_error_is_not_retried(self, slept):
        client = make_client([Response(400, text="bad field")])
        with pytest.raises(BambooHrApiError, match="bad field"):
            client.get("meta/fields")

        assert len(client._session.calls) == 1
