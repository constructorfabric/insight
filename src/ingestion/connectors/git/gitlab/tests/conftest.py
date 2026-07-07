"""Shared fixtures/helpers for source_gitlab unit tests.

All tests are offline: HTTP is stubbed either via FakeResponse objects or by
monkeypatching the concurrency-module transport functions (walk_window /
paginate / imap_bounded / imap_stream). No network, no credentials.
"""

from __future__ import annotations

from typing import Any

from airbyte_cdk.models import SyncMode

BASE = {
    "base_url": "https://gl.example",
    "token": "tok",
    "tenant_id": "T",
    "source_id": "S",
}


class FakeResponse:
    """Duck-typed requests.Response for GitlabStream.parse_response/next_page_token."""

    def __init__(self, payload: Any = None, status_code: int = 200,
                 links: dict | None = None, text: str = ""):
        self._payload = payload
        self.status_code = status_code
        self.links = links or {}
        self.url = "https://gl.example/api/v4/x"
        self.text = text

    def json(self) -> Any:
        return self._payload


class FakeParent:
    """Projects-parent stub: (slice, records) pairs, as in test_substream."""

    def __init__(self, scopes: list[tuple[dict, list[Any]]]) -> None:
        self._scopes = scopes

    def stream_slices(self, **kwargs: Any) -> list[dict]:
        return [s for s, _ in self._scopes]

    def read_records(self, sync_mode: SyncMode = SyncMode.full_refresh,
                     stream_slice: Any = None, **kwargs: Any) -> list[Any]:
        for s, records in self._scopes:
            if s == stream_slice:
                return records
        return []


class FakeBranches:
    """Branches-stream stub keyed by project id."""

    def __init__(self, by_project: dict[Any, list[dict]]) -> None:
        self._by_project = by_project

    def read_records(self, sync_mode: SyncMode = SyncMode.full_refresh,
                     stream_slice: Any = None, **kwargs: Any):
        project = (stream_slice or {}).get("parent") or {}
        yield from self._by_project.get(project.get("id"), [])


def fake_walk_window_yielding(raws: list[dict]):
    """A concurrency.walk_window replacement that still drives path_fn/params_fn
    and envelopes each raw record — so the stream-side lambdas get covered."""

    calls: list[dict] = []

    def _fake(*, strategy, base_slice, url_base, path_fn, params_fn,
              envelope_fn, headers, gate, skippable, timeout=None, **kw):
        applied = dict(base_slice)
        calls.append({
            "base_slice": dict(base_slice),
            "path": path_fn(applied),
            "params": dict(params_fn(applied)),
            "skippable": skippable,
        })
        for raw in raws:
            yield envelope_fn(raw, applied)

    _fake.calls = calls
    return _fake


def fake_paginate_yielding(raws: list[dict]):
    """A concurrency.paginate replacement enveloping each raw record."""

    calls: list[dict] = []

    def _fake(gate, *, url_base, path, params, envelope_fn, headers,
              skippable, timeout=None, **kw):
        calls.append({"path": path, "params": dict(params), "skippable": skippable})
        for raw in raws:
            yield envelope_fn(raw)

    _fake.calls = calls
    return _fake


def fake_imap_bounded(gate, tasks, fn):
    """Sequential, deterministic imap_bounded."""
    for task in tasks:
        yield fn(task)


def fake_imap_stream(gate, tasks, fn):
    """Sequential, deterministic imap_stream preserving the Done protocol."""
    from source_gitlab.streams import concurrency

    for task in tasks:
        gen = fn(task)
        while True:
            try:
                record = next(gen)
            except StopIteration as stop:
                yield task, concurrency.Done(stop.value)
                break
            yield task, record
