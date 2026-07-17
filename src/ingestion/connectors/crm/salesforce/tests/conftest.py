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

# Small describe-derived schema: standard fields + one custom (``__c``) field.
ACCOUNT_SCHEMA = {
    "$schema": "http://json-schema.org/draft-07/schema#",
    "type": "object",
    "additionalProperties": True,
    "properties": {
        "Id": {"type": ["string", "null"]},
        "Name": {"type": ["string", "null"]},
        "SystemModstamp": {"type": ["string", "null"], "format": "date-time"},
        "Custom__c": {"type": ["string", "null"]},
    },
}
CUSTOM_FIELDS = frozenset({"Custom__c"})


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


_DEFAULT_SCHEMA = object()  # sentinel: allows passing schema=None explicitly


def make_stream(
    cls=RestSalesforceStream,
    stream_name: str = "Account",
    schema: Any = _DEFAULT_SCHEMA,
    pk: str = "Id",
    sf: Salesforce | None = None,
    **extra: Any,
):
    """Construct a stream with a real (offline) Salesforce client."""
    sf = sf or make_sf()
    kwargs = dict(
        sf_api=sf,
        pk=pk,
        stream_name=stream_name,
        message_repository=InMemoryMessageRepository(),
        schema=dict(ACCOUNT_SCHEMA) if schema is _DEFAULT_SCHEMA else schema,
        authenticator=SalesforceAuthenticator(sf._token_provider),
        tenant_id=TENANT,
        source_id=SOURCE,
        custom_field_names=CUSTOM_FIELDS,
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
