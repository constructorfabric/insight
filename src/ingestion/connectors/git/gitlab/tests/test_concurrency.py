from __future__ import annotations

import threading
import time
from typing import Any

import pytest

from source_gitlab.streams import concurrency
from source_gitlab.streams.concurrency import (
    Done,
    OrderedPrefix,
    RequestGate,
    backoff_time,
    imap_bounded,
    imap_stream,
    should_retry,
    walk_window,
)
from source_gitlab.streams.errors import GitlabApiError, UnwindowableWindow
from source_gitlab.streams.windowing import UpdatedAtWindowing


class _Resp:
    def __init__(self, status: int = 200, headers: dict | None = None) -> None:
        self.status_code = status
        self.headers = headers or {}


def gate(workers: int = 4) -> RequestGate:
    return RequestGate(workers)


class TestRetryClassification:
    def test_should_retry_codes(self) -> None:
        assert all(should_retry(_Resp(c)) for c in (429, 500, 502, 503, 504))
        assert not any(should_retry(_Resp(c)) for c in (200, 400, 401, 403, 404))

    def test_backoff_retry_after(self) -> None:
        assert backoff_time(_Resp(429, {"Retry-After": "7"})) == 7.0

    def test_backoff_ratelimit_reset(self) -> None:
        reset = time.time() + 30
        got = backoff_time(_Resp(429, {"RateLimit-Reset": str(reset)}))
        assert got is not None and 20 <= got <= 31

    def test_backoff_default_and_none(self) -> None:
        assert backoff_time(_Resp(429)) == 60.0
        assert backoff_time(_Resp(503)) is None


class TestRequestGate:
    def test_note_throttle_pauses_then_resumes(self) -> None:
        g = gate()
        g.note_throttle(0.25)
        start = time.monotonic()
        with g.request_slot():
            pass
        assert time.monotonic() - start >= 0.2

    def test_note_throttle_keeps_the_longest(self) -> None:
        g = gate()
        g.note_throttle(0.4)
        g.note_throttle(0.1)
        with g._lock:
            remaining = g._resume_at - time.monotonic()
        assert remaining > 0.3

    def test_slot_bounds_concurrency(self) -> None:
        g = gate(2)
        live = []
        max_live = [0]
        lock = threading.Lock()

        def work() -> None:
            with g.request_slot():
                with lock:
                    live.append(1)
                    max_live[0] = max(max_live[0], len(live))
                time.sleep(0.05)
                with lock:
                    live.pop()

        threads = [threading.Thread(target=work) for _ in range(6)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        assert max_live[0] <= 2


class TestImapBounded:
    def test_yields_all_results(self) -> None:
        results = set(imap_bounded(gate(), range(50), lambda x: x * 2))
        assert results == {x * 2 for x in range(50)}

    def test_bounded_concurrency(self) -> None:
        g = gate(3)
        live = [0]
        peak = [0]
        lock = threading.Lock()

        def work(_: Any) -> int:
            with lock:
                live[0] += 1
                peak[0] = max(peak[0], live[0])
            time.sleep(0.02)
            with lock:
                live[0] -= 1
            return 1

        list(imap_bounded(g, range(30), work))
        assert peak[0] <= 3

    def test_propagates_worker_exception(self) -> None:
        def work(x: int) -> int:
            if x == 5:
                raise GitlabApiError("boom")
            return x

        with pytest.raises(GitlabApiError):
            list(imap_bounded(gate(), range(50), work))

    def test_propagates_enumeration_exception(self) -> None:
        def tasks() -> Any:
            yield 1
            raise ValueError("enum boom")

        with pytest.raises(ValueError):
            list(imap_bounded(gate(), tasks(), lambda x: x))


class TestImapStream:
    def test_streams_records_and_done_values(self) -> None:
        def fn(task: int) -> Any:
            for i in range(task):
                yield (task, i)
            return f"done-{task}"

        items = list(imap_stream(gate(), [1, 2, 3], fn))
        records = sorted(it for _, it in items if not isinstance(it, Done))
        dones = {t: it.value for t, it in items if isinstance(it, Done)}
        assert records == [(1, 0), (2, 0), (2, 1), (3, 0), (3, 1), (3, 2)]
        assert dones == {1: "done-1", 2: "done-2", 3: "done-3"}

    def test_empty_generator_yields_only_done(self) -> None:
        def fn(task: int) -> Any:
            return f"empty-{task}"
            yield  # noqa: W0101 — makes fn a generator

        items = list(imap_stream(gate(), [1, 2], fn))
        assert all(isinstance(it, Done) for _, it in items)
        assert {t: it.value for t, it in items} == {1: "empty-1", 2: "empty-2"}

    def test_records_precede_their_done(self) -> None:
        def fn(task: int) -> Any:
            for i in range(5):
                yield (task, i)
            return None

        done_seen: set[int] = set()
        for task, item in imap_stream(gate(2), range(8), fn):
            if isinstance(item, Done):
                done_seen.add(task)
            else:
                assert task not in done_seen

    def test_bounded_concurrency(self) -> None:
        g = gate(3)
        live = [0]
        peak = [0]
        lock = threading.Lock()

        def fn(task: int) -> Any:
            with lock:
                live[0] += 1
                peak[0] = max(peak[0], live[0])
            time.sleep(0.02)
            yield (task, 0)
            with lock:
                live[0] -= 1

        list(imap_stream(g, range(20), fn))
        assert 2 <= peak[0] <= 3

    def test_propagates_worker_exception(self) -> None:
        def fn(task: int) -> Any:
            if task == 5:
                raise GitlabApiError("boom")
            yield (task, 0)

        with pytest.raises(GitlabApiError):
            list(imap_stream(gate(), range(50), fn))

    def test_propagates_enumeration_exception(self) -> None:
        def tasks() -> Any:
            yield 1
            raise ValueError("enum boom")

        def fn(task: int) -> Any:
            yield (task, 0)

        with pytest.raises(ValueError):
            list(imap_stream(gate(), tasks(), fn))


class TestOrderedPrefix:
    def test_in_order_releases_immediately(self) -> None:
        p = OrderedPrefix()
        assert list(p.complete(0, "a")) == ["a"]
        assert list(p.complete(1, "b")) == ["b"]

    def test_out_of_order_buffers_until_contiguous(self) -> None:
        p = OrderedPrefix()
        assert list(p.complete(2, "c")) == []
        assert list(p.complete(1, "b")) == []
        assert list(p.complete(0, "a")) == ["a", "b", "c"]


class TestWalkWindow:
    def _run(self, responses: dict[Any, list[_Resp]], **kw: Any) -> list[dict]:
        strat = UpdatedAtWindowing.__new__(UpdatedAtWindowing)
        return list(
            walk_window(
                strategy=strat,
                base_slice={},
                url_base="https://x/api/v4/",
                path_fn=lambda s: "merge_requests",
                params_fn=lambda s: {"updated_after": s.get("updated_after")},
                envelope_fn=lambda raw, s: raw,
                headers={},
                gate=gate(),
                skippable=frozenset(),
                **kw,
            )
        )

    def test_paginates_a_window(self, monkeypatch: Any) -> None:
        pages = iter(
            [
                _page([{"updated_at": "2026-01-01T00:00:00Z"}], nxt="p2"),
                _page([{"updated_at": "2026-01-02T00:00:00Z"}], nxt=None),
            ]
        )
        monkeypatch.setattr(concurrency, "send", lambda *a, **k: next(pages))
        out = self._run({})
        assert [r["updated_at"] for r in out] == [
            "2026-01-01T00:00:00Z",
            "2026-01-02T00:00:00Z",
        ]

    def test_soft_cap_splits_and_continues(self, monkeypatch: Any) -> None:
        monkeypatch.setattr(concurrency, "SOFT_PAGE_LIMIT", 2)

        def fake_send(g: Any, url: str, headers: Any, params: Any, **k: Any) -> _Resp:
            if params.get("updated_after") is None:
                return _page([{"updated_at": "2026-03-01T00:00:00Z"}], nxt="more")
            return _page([{"updated_at": "2026-02-01T00:00:00Z"}], nxt=None)

        monkeypatch.setattr(concurrency, "send", fake_send)
        out = self._run({})
        # window 1 streamed (capped after 2 next-pages), then split window fetched
        assert len(out) >= 3
        assert any(r["updated_at"] == "2026-02-01T00:00:00Z" for r in out)

    def test_offset_400_with_no_progress_fails_loud(self, monkeypatch: Any) -> None:
        monkeypatch.setattr(
            concurrency, "send", lambda *a, **k: _Resp400("offset exceeds maximum")
        )
        with pytest.raises(UnwindowableWindow):
            self._run({})


def _page(records: list[dict], *, nxt: str | None) -> Any:
    return _PageResp(records, nxt)


class _PageResp:
    def __init__(self, records: list[dict], nxt: str | None) -> None:
        self.status_code = 200
        self._records = records
        self.links = {"next": {"url": nxt}} if nxt else {}
        self.text = ""

    def json(self) -> list[dict]:
        return self._records


class _Resp400:
    def __init__(self, text: str) -> None:
        self.status_code = 400
        self.text = text
        self.links: dict = {}

    def json(self) -> list:
        return []
