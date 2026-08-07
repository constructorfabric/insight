from __future__ import annotations

from collections.abc import Mapping
from typing import Any

import pytest

TENANT = "T"
SOURCE = "S"


class FakeClient:
    """Stands in for BambooClient: answers each path from a canned payload and
    records what was asked for."""

    def __init__(self, responses: Mapping[str, Any] | None = None) -> None:
        self._responses = dict(responses or {})
        self.calls: list[tuple[str, str, Any]] = []

    def get(self, path: str, params: Mapping[str, Any] | None = None) -> Any:
        self.calls.append(("GET", path, params))
        return self._payload(path, None)

    def post(self, path: str, body: Mapping[str, Any]) -> Any:
        self.calls.append(("POST", path, body))
        return self._payload(path, body)

    def _payload(self, path: str, body: Mapping[str, Any] | None) -> Any:
        if path not in self._responses:
            raise AssertionError(f"unexpected request path: {path}")

        payload = self._responses[path]
        if isinstance(payload, Exception):
            raise payload
        if callable(payload):
            return payload(body)
        return payload


def custom_report(rows: list[Any], omit: set[str] | None = None, declare_columns: bool = True):
    """A custom-report responder shaped like BambooHR's: it echoes the requested
    fields as report columns, minus anything `omit` says the credential cannot
    read."""
    withheld = omit or set()

    def respond(body: Mapping[str, Any]) -> dict[str, Any]:
        answered = [name for name in body["fields"] if name not in withheld]
        visible = [
            {key: value for key, value in row.items() if key not in withheld}
            if isinstance(row, Mapping)
            else row
            for row in rows
        ]

        payload: dict[str, Any] = {"title": body["title"], "employees": visible}
        if declare_columns:
            payload["fields"] = [{"id": name, "type": "text", "name": name} for name in answered]
        return payload

    return respond


def meta_field(field_id: int, alias: str | None = None, **extra: Any) -> dict[str, Any]:
    field: dict[str, Any] = {"id": field_id, "name": f"Field {field_id}", "type": "text"}
    if alias is not None:
        field["alias"] = alias
    field.update(extra)
    return field


@pytest.fixture
def no_sleep(monkeypatch):
    monkeypatch.setattr("source_bamboohr.client.time.sleep", lambda *_a, **_k: None)
    monkeypatch.setattr("source_bamboohr.client.random.random", lambda: 0.0)
