"""Shared fixtures/helpers for source_salesforce unit tests.

All tests are offline: HTTP is stubbed either with duck-typed FakeResponse
objects (streams/api call sites only touch ``.json()`` / ``.status_code`` /
``.text``) or with real ``requests.Response`` objects built in memory for the
rate-limiting handler, which does ``isinstance(response, requests.Response)``
checks. No network, no credentials.
"""

from __future__ import annotations

import json
from typing import Any
from unittest.mock import Mock

import pytest
import requests
from airbyte_cdk.sources.message import InMemoryMessageRepository
from source_salesforce.api import Salesforce, SalesforceAuthenticator
from source_salesforce.streams import IncrementalRestSalesforceStream, RestSalesforceStream

TENANT = "T"
SOURCE = "S"
INSTANCE_URL = "https://insight.example.my.salesforce.com"

CONFIG = {
    "salesforce_instance_url": INSTANCE_URL,
    "salesforce_client_id": "cid",
    "salesforce_client_secret": "sec",
    "salesforce_start_date": "2024-01-01T00:00:00Z",
    "insight_tenant_id": TENANT,
    "insight_source_id": SOURCE,
}

# Fields describe reports for the Account sobject in these tests: declared
# standard fields plus one custom (``__c``) field.
ACCOUNT_FIELDS = ("Id", "Name", "SystemModstamp", "Custom__c")


class FakeResponse:
    """Minimal stand-in for requests.Response as consumed by api/streams code.

    login/describe/parse_response/next_page_token only touch .json(),
    .status_code and .text.
    """

    def __init__(
        self, payload: Any = None, status_code: int = 200, url: str = f"{INSTANCE_URL}/services/data/vXX.X/queryAll"
    ):
        self._payload = payload
        self.status_code = status_code
        self.url = url

    @property
    def text(self) -> str:
        if isinstance(self._payload, Exception):
            return "<non-json body>"
        return json.dumps(self._payload)

    def json(self) -> Any:
        if isinstance(self._payload, Exception):
            raise self._payload
        return self._payload


def make_http_response(
    status_code: int = 200,
    payload: Any = None,
    content: bytes | None = None,
    url: str = f"{INSTANCE_URL}/services/data/vXX.X/queryAll",
) -> requests.Response:
    """Build a real requests.Response in memory.

    Needed for SalesforceErrorHandler, which does isinstance() checks that a
    duck-typed fake would not pass.
    """
    resp = requests.Response()
    resp.status_code = status_code
    if content is not None:
        resp._content = content
    else:
        resp._content = json.dumps(payload if payload is not None else {}).encode()
    resp.request = requests.PreparedRequest()
    resp.request.url = url
    resp.url = url
    return resp


def make_sf(**overrides: Any) -> Salesforce:
    """Real Salesforce client, never logged in (no HTTP happens at init)."""
    kwargs = {"instance_url": INSTANCE_URL, "client_id": "cid", "client_secret": "sec"}
    kwargs.update(overrides)
    return Salesforce(**kwargs)


_DEFAULT_FIELDS = object()  # sentinel: allows passing sf_fields=None explicitly


def make_stream(
    cls=RestSalesforceStream,
    stream_name: str = "Account",
    sf_fields: Any = _DEFAULT_FIELDS,
    pk: str = "Id",
    sf: Salesforce | None = None,
    **extra: Any,
):
    """Construct a stream with a real (offline) Salesforce client.

    ``sf_fields`` stubs the describe-reported field list the stream builds SOQL
    from, and marks the sobject available; ``None`` leaves the real
    (network-bound) lookups in place.
    """
    sf = sf or make_sf()
    if sf_fields is not None:
        fields = ACCOUNT_FIELDS if sf_fields is _DEFAULT_FIELDS else tuple(sf_fields)
        sf.field_names = Mock(return_value=fields)
        sf.is_queryable = Mock(return_value=True)

    kwargs = dict(
        sf_api=sf,
        pk=pk,
        stream_name=stream_name,
        message_repository=InMemoryMessageRepository(),
        authenticator=SalesforceAuthenticator(sf._token_provider),
        tenant_id=TENANT,
        source_id=SOURCE,
    )
    kwargs.update(extra)
    return cls(**kwargs)


def make_incremental(**extra: Any) -> IncrementalRestSalesforceStream:
    extra.setdefault("replication_key", "SystemModstamp")
    return make_stream(cls=IncrementalRestSalesforceStream, **extra)


@pytest.fixture
def sf() -> Salesforce:
    return make_sf()


@pytest.fixture
def stream() -> RestSalesforceStream:
    return make_stream()


@pytest.fixture
def incremental_stream() -> IncrementalRestSalesforceStream:
    return make_incremental()
