from __future__ import annotations

import json
from collections.abc import Mapping
from typing import Any

import pytest
from source_bamboohr.config import BambooConfig
from source_bamboohr.streams.employees import CustomReport, ReportColumn
from source_bamboohr.streams.meta_fields import MetaField

TENANT = "T"
SOURCE = "S"


def config(**overrides: Any) -> dict[str, Any]:
    base = BambooConfig(
        insight_tenant_id=TENANT, insight_source_id=SOURCE, bamboohr_api_key="key", bamboohr_domain="acme"
    )
    return {**base.model_dump(mode="json", exclude_none=True), **overrides}


class FakeClient:
    """Stands in for BambooClient: answers each path with a canned payload as a
    JSON body (bytes are sent as they are) and records what was asked for."""

    def __init__(self, responses: Mapping[str, Any] | None = None) -> None:
        self._responses = dict(responses or {})
        self.calls: list[tuple[str, str, Any]] = []

    def get(self, path: str, params: Mapping[str, Any] | None = None) -> bytes:
        self.calls.append(("GET", path, params))
        return self._body(path, None)

    def post(self, path: str, body: Mapping[str, Any]) -> bytes:
        self.calls.append(("POST", path, body))
        return self._body(path, body)

    def _body(self, path: str, body: Mapping[str, Any] | None) -> bytes:
        if path not in self._responses:
            raise AssertionError(f"unexpected request path: {path}")

        payload = self._responses[path]
        if isinstance(payload, Exception):
            raise payload
        if callable(payload):
            payload = payload(body)
        if isinstance(payload, bytes):
            return payload
        return json.dumps(payload).encode()


def custom_report(rows: list[Any], omit: set[str] | None = None, declare_columns: bool = True):
    """A custom-report responder shaped like BambooHR's: it echoes the requested
    fields as report columns, minus anything `omit` says the credential cannot
    read."""
    withheld = omit or set()

    def respond(body: Mapping[str, Any]) -> dict[str, Any]:
        answered = [name for name in body["fields"] if name not in withheld]
        visible = [
            {key: value for key, value in row.items() if key not in withheld} if isinstance(row, Mapping) else row
            for row in rows
        ]

        columns = [ReportColumn(id=name) for name in answered] if declare_columns else None
        return CustomReport(employees=visible, fields=columns).model_dump(mode="json", exclude_none=True)

    return respond


def meta_field(field_id: int, alias: str = "", deprecated: bool = False) -> dict[str, Any]:
    field = MetaField(id=str(field_id), alias=alias, deprecated=deprecated)
    return field.model_dump(mode="json", exclude_defaults=True)


@pytest.fixture
def no_sleep(monkeypatch):
    monkeypatch.setattr("source_bamboohr.client.time.sleep", lambda *_a, **_k: None)
    monkeypatch.setattr("source_bamboohr.client.random.random", lambda: 0.0)
