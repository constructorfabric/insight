from __future__ import annotations

import random
import re
import time
from collections.abc import Mapping
from email.utils import parsedate_to_datetime
from typing import Any

import requests
from requests.auth import HTTPBasicAuth

DNS_LABEL = re.compile(r"[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?")

MAX_ATTEMPTS = 6
RETRYABLE_STATUSES = frozenset({408, 429, 500, 502, 503, 504})
TIMEOUT = (10, 120)
MAX_RETRY_DELAY = 300.0
MAX_BACKOFF = 60.0

# BambooHR answers 401 for a rejected key and 403 for a key whose user lacks the
# permission; neither is retryable and both are configuration facts the operator
# has to act on, so the message names the remedy instead of the status alone.
AUTH_HINTS = {
    401: (
        "the API key was rejected — it is invalid, expired, or revoked. "
        "Regenerate it under Account > API Keys and update bamboohr_api_key."
    ),
    403: (
        "the API key lacks permission for this resource — grant its user access "
        "to employee data and field metadata in BambooHR."
    ),
}


class BambooHrApiError(RuntimeError):
    def __init__(self, status_code: int, url: str, detail: str) -> None:
        super().__init__(f"BambooHR API returned {status_code} for {url}: {detail[:500]}")
        self.status_code = status_code
        self.url = url
        self.detail = detail


class BambooHrAuthError(BambooHrApiError):
    pass


class BambooHrDomainError(ValueError):
    pass


def api_base_url(domain: str) -> str:
    """Base URL for a BambooHR subdomain.

    The subdomain reaches the URL as a host component while the session carries
    the API key as HTTP Basic credentials, so anything that can terminate the
    host — a dot, a delimiter, userinfo, a port — would send the key to a host
    of the caller's choosing. Only a bare DNS label is accepted.
    """
    label = (domain or "").strip()
    if not DNS_LABEL.fullmatch(label):
        raise BambooHrDomainError(
            f"bamboohr_domain must be a bare BambooHR subdomain such as 'acme' "
            f"(letters, digits and hyphens only), not {domain!r}."
        )

    return f"https://{label}.bamboohr.com/api/v1/"


class BambooClient:
    def __init__(self, domain: str, api_key: str) -> None:
        self._base_url = api_base_url(domain).rstrip("/") + "/"
        self._session = requests.Session()
        self._session.auth = HTTPBasicAuth(api_key, "x")
        self._session.headers.update({"Accept": "application/json"})

    def get(self, path: str, params: Mapping[str, Any] | None = None) -> Any:
        return self._json(self._request("GET", path, params=params, body=None))

    def post(self, path: str, body: Mapping[str, Any]) -> Any:
        return self._json(self._request("POST", path, params=None, body=body))

    def _request(
        self,
        method: str,
        path: str,
        *,
        params: Mapping[str, Any] | None,
        body: Mapping[str, Any] | None,
    ) -> requests.Response:
        url = f"{self._base_url}{path.lstrip('/')}"
        attempt = 0

        while True:
            try:
                response = self._session.request(method, url, params=params, json=body, timeout=TIMEOUT)
            except requests.RequestException:
                if attempt >= MAX_ATTEMPTS - 1:
                    raise
                time.sleep(_backoff(attempt) + random.random())
                attempt += 1
                continue

            if response.status_code < 400:
                return response
            if response.status_code in RETRYABLE_STATUSES and attempt < MAX_ATTEMPTS - 1:
                time.sleep(_retry_delay(response, attempt) + random.random())
                attempt += 1
                continue

            raise _api_error(response)

    def _json(self, response: requests.Response) -> Any:
        try:
            return response.json()
        except ValueError as exc:
            raise RuntimeError(f"BambooHR returned invalid JSON from {response.url}") from exc


def _api_error(response: requests.Response) -> BambooHrApiError:
    hint = AUTH_HINTS.get(response.status_code)
    if hint is not None:
        return BambooHrAuthError(response.status_code, response.url, hint)
    return BambooHrApiError(response.status_code, response.url, response.text)


def _retry_delay(response: requests.Response, attempt: int) -> float:
    retry_after = response.headers.get("Retry-After")
    if not retry_after:
        return _backoff(attempt)

    try:
        return min(MAX_RETRY_DELAY, max(0.0, float(retry_after)))
    except ValueError:
        pass

    try:
        retry_at = parsedate_to_datetime(retry_after).timestamp()
    except (TypeError, ValueError, OverflowError):
        return _backoff(attempt)

    return min(MAX_RETRY_DELAY, max(0.0, retry_at - time.time()))


def _backoff(attempt: int) -> float:
    return min(MAX_BACKOFF, 2.0**attempt)
