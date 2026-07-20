"""Shared fixtures for source_hubspot unit tests.

All tests are offline: HTTP is stubbed by swapping the CDK ``HttpClient``
on a constructed stream (or ``Hubspot`` api client) for a ``FakeHttpClient``
that replays queued duck-typed ``FakeResponse`` objects. No network, no
credentials.
"""

from __future__ import annotations

import json
from collections.abc import Iterable, Mapping
from typing import Any

import pendulum
import pytest
from source_hubspot.streams import (
    CrmArchivedListStream,
    CrmSearchStream,
    HubspotStream,
    OwnersArchivedStream,
    OwnersStream,
)

TENANT = "T"
SOURCE = "S"
START = pendulum.datetime(2024, 1, 1, tz="UTC")


class FakeResponse:
    """Minimal stand-in for requests.Response as consumed by streams/api.

    The code under test only touches ``.json()``, ``.ok``, ``.status_code``,
    ``.url`` and ``.text``.
    """

    def __init__(
        self,
        payload: Any = None,
        status_code: int = 200,
        url: str = "https://api.hubapi.com/x",
        text: str | None = None,
    ):
        self._payload = payload
        self.status_code = status_code
        self.url = url
        if text is not None:
            self.text = text
        else:
            self.text = json.dumps(payload) if payload is not None else ""

    @property
    def ok(self) -> bool:
        return self.status_code < 400

    def json(self) -> Any:
        if isinstance(self._payload, Exception):
            raise self._payload
        return self._payload


class FakeHttpClient:
    """Duck-typed CDK HttpClient: records calls, replays queued responses.

    Each queued item is either a FakeResponse (returned as ``(None, resp)``,
    mirroring the ``(request, response)`` tuple of the real client) or an
    Exception (raised).
    """

    def __init__(self, responses: Iterable[Any] = ()):
        self.responses = list(responses)
        self.calls: list[dict] = []

    def send_request(
        self,
        method: str,
        url: str,
        headers: Mapping[str, str] | None = None,
        params: Mapping[str, Any] | None = None,
        json: Any = None,
        request_kwargs: Mapping[str, Any] | None = None,
        **kwargs: Any,
    ):
        self.calls.append(
            {"method": method, "url": url, "headers": dict(headers or {}), "params": dict(params or {}), "json": json}
        )
        if not self.responses:
            raise AssertionError("FakeHttpClient ran out of queued responses")
        item = self.responses.pop(0)
        if isinstance(item, Exception):
            raise item
        return None, item


class FakeHubspot:
    """Describe-time api stub: canned property descriptors, no HTTP."""

    def __init__(self, names: Iterable[str] = ("amount", "my_custom"), custom: Iterable[str] = ("my_custom",)):
        self._names = tuple(names)
        self._custom = frozenset(custom)

    def property_names(self, object_type: str) -> tuple:
        return self._names

    def custom_property_names(self, object_type: str) -> frozenset:
        return self._custom

    def generate_schema(self, object_type: str) -> Mapping[str, Any]:
        return {
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "additionalProperties": True,
            "properties": {"id": {"type": ["string", "null"]}, "properties_amount": {"type": ["string", "null"]}},
        }


def make_stream(cls, stream_name: str, hubspot: Any = None, **overrides: Any) -> HubspotStream:
    """Construct a stream with test scope and a FakeHubspot describe stub."""
    kwargs = dict(
        stream_name=stream_name,
        hubspot_api=hubspot if hubspot is not None else FakeHubspot(),
        access_token="pat-test-token",
        tenant_id=TENANT,
        source_id=SOURCE,
        start_date=START,
    )
    kwargs.update(overrides)
    return cls(**kwargs)


def wire(stream: HubspotStream, responses: Iterable[Any]) -> FakeHttpClient:
    """Swap the stream's HttpClient (and the association fetcher's, which
    shares it) for a FakeHttpClient replaying ``responses`` in order."""
    fake = FakeHttpClient(responses)
    stream._http_client = fake
    if stream._associations is not None:
        stream._associations._http_client = fake
    return fake


@pytest.fixture
def deals_stream() -> CrmSearchStream:
    # deals: associations = [companies, contacts]
    return make_stream(CrmSearchStream, "deals")


@pytest.fixture
def companies_stream() -> CrmSearchStream:
    # companies: no associations — simplest search stream
    return make_stream(CrmSearchStream, "companies")


@pytest.fixture
def owners_stream() -> OwnersStream:
    return make_stream(OwnersStream, "owners")


@pytest.fixture
def owners_archived_stream() -> OwnersArchivedStream:
    return make_stream(OwnersArchivedStream, "owners_archived")


@pytest.fixture
def companies_archived_stream() -> CrmArchivedListStream:
    # companies_archived: no associations — keeps batch_read tests focused
    return make_stream(CrmArchivedListStream, "companies_archived")
