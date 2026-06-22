from __future__ import annotations

import queue
import random
import threading
import time
from collections import deque
from collections.abc import (
    Callable,
    Generator,
    Iterable,
    Iterator,
    Mapping,
    MutableMapping,
)
from concurrent.futures import FIRST_COMPLETED, Future, ThreadPoolExecutor, wait
from contextlib import contextmanager
from typing import Any

import requests

from source_gitlab.streams.errors import (
    GitlabApiError,
    GitlabAuthError,
    WindowTooLarge,
)

SOFT_PAGE_LIMIT = 490
REQUEST_TIMEOUT = 120.0
_MAX_5XX_RETRIES = 6
_MAX_429_RETRIES = 50

_local = threading.local()


def session() -> requests.Session:
    existing: requests.Session | None = getattr(_local, "session", None)
    if existing is not None:
        return existing
    created = requests.Session()
    adapter = requests.adapters.HTTPAdapter(pool_connections=4, pool_maxsize=4)
    created.mount("https://", adapter)
    created.mount("http://", adapter)
    _local.session = created
    return created


def should_retry(response: requests.Response) -> bool:
    return response.status_code == 429 or response.status_code in (500, 502, 503, 504)


def backoff_time(response: requests.Response) -> float | None:
    if response.status_code == 429:
        retry_after = response.headers.get("Retry-After")
        if retry_after:
            try:
                return max(float(retry_after), 1.0)
            except ValueError:
                pass
        reset = response.headers.get("RateLimit-Reset")
        if reset:
            try:
                return max(float(reset) - time.time(), 1.0)
            except ValueError:
                pass
        return 60.0
    return None


def _expo(attempt: int) -> float:
    cap = min(2.0**attempt, 60.0)
    return cap / 2 + random.uniform(0, cap / 2)


class RequestGate:
    def __init__(self, max_workers: int) -> None:
        self.max_workers = max_workers
        self._sem = threading.BoundedSemaphore(max_workers)
        self._lock = threading.Lock()
        self._resume_at = 0.0
        self._executor: ThreadPoolExecutor | None = None
        self._executor_lock = threading.Lock()

    @property
    def executor(self) -> ThreadPoolExecutor:
        if self._executor is None:
            with self._executor_lock:
                if self._executor is None:
                    self._executor = ThreadPoolExecutor(
                        max_workers=self.max_workers, thread_name_prefix="gitlab"
                    )
        return self._executor

    def shutdown(self) -> None:
        if self._executor is not None:
            self._executor.shutdown(wait=False)

    def _await_resume(self) -> None:
        while True:
            with self._lock:
                wait_for = self._resume_at - time.monotonic()
            if wait_for <= 0:
                return
            time.sleep(min(wait_for, 1.0))

    def note_throttle(self, seconds: float) -> None:
        with self._lock:
            self._resume_at = max(self._resume_at, time.monotonic() + max(seconds, 0.0))

    @contextmanager
    def request_slot(self) -> Iterator[None]:
        self._await_resume()
        self._sem.acquire()
        try:
            yield
        finally:
            self._sem.release()


def send(
    gate: RequestGate,
    url: str,
    headers: Mapping[str, str],
    params: Mapping[str, Any],
    *,
    timeout: float = REQUEST_TIMEOUT,
) -> requests.Response:
    retries_5xx = 0
    retries_429 = 0
    retries_net = 0
    while True:
        try:
            with gate.request_slot():
                response = session().get(
                    url, headers=headers, params=params, timeout=timeout
                )
        except requests.RequestException as exc:
            retries_net += 1
            if retries_net > _MAX_5XX_RETRIES:
                raise GitlabApiError(f"network error after retries on {url}: {exc}") from exc
            time.sleep(_expo(retries_net))
            continue
        if not should_retry(response):
            return response
        wait_for = backoff_time(response)
        if response.status_code == 429:
            retries_429 += 1
            if retries_429 > _MAX_429_RETRIES:
                raise GitlabApiError(f"rate limited past retry budget on {url}")
            gate.note_throttle(wait_for or 60.0)
            continue
        retries_5xx += 1
        if retries_5xx > _MAX_5XX_RETRIES:
            return response
        time.sleep(wait_for or _expo(retries_5xx))


def classify(
    response: requests.Response,
    *,
    skippable: frozenset[int],
    first_page: bool,
    stream_slice: Mapping[str, Any],
) -> list[Mapping[str, Any]] | None:
    code = response.status_code
    skip_disabled = stream_slice.get("skip_404") is False
    if first_page and code in skippable and not skip_disabled:
        return None
    if code in (401, 403):
        raise GitlabAuthError(f"GitLab auth error ({code}): {response.text[:200]}")
    if code >= 400:
        raise GitlabApiError(f"Unexpected HTTP {code} on {response.url}: {response.text[:200]}")
    data = response.json()
    return data if isinstance(data, list) else [data]


def paginate(
    gate: RequestGate,
    *,
    url_base: str,
    path: str,
    params: Mapping[str, Any],
    envelope_fn: Callable[[Mapping[str, Any]], Mapping[str, Any]],
    headers: Mapping[str, str],
    skippable: frozenset[int],
    timeout: float = REQUEST_TIMEOUT,
) -> Iterator[Mapping[str, Any]]:
    url: str | None = url_base + path
    page_params: Mapping[str, Any] = dict(params)
    first_page = True
    while url is not None:
        response = send(gate, url, headers, page_params, timeout=timeout)
        records = classify(
            response, skippable=skippable, first_page=first_page, stream_slice={}
        )
        if records is None:
            return
        for raw in records:
            yield envelope_fn(raw)
        url = response.links.get("next", {}).get("url")
        page_params = {}
        first_page = False


def walk_window(
    *,
    strategy: Any,
    base_slice: Mapping[str, Any],
    url_base: str,
    path_fn: Callable[[Mapping[str, Any]], str],
    params_fn: Callable[[Mapping[str, Any]], Mapping[str, Any]],
    envelope_fn: Callable[[Mapping[str, Any], Mapping[str, Any]], Mapping[str, Any]],
    headers: Mapping[str, str],
    gate: RequestGate,
    skippable: frozenset[int],
    timeout: float = REQUEST_TIMEOUT,
) -> Iterator[Mapping[str, Any]]:
    windows: deque[Mapping[str, Any]] = deque([strategy._window_initial(base_slice)])
    while windows:
        window = windows.popleft()
        applied = strategy._window_apply(base_slice, window)
        url: str | None = url_base + path_fn(applied)
        params: Mapping[str, Any] = params_fn(applied)
        page_count = 0
        first_page = True
        last_value: str | None = None
        try:
            while url is not None:
                response = send(gate, url, headers, params, timeout=timeout)
                if response.status_code == 400 and "offset" in (response.text or "").lower():
                    raise WindowTooLarge
                records = classify(
                    response, skippable=skippable, first_page=first_page, stream_slice=applied
                )
                if records is None:
                    break
                for raw in records:
                    enveloped = envelope_fn(raw, applied)
                    value = strategy._window_value(enveloped)
                    if value:
                        last_value = value
                    yield enveloped
                nxt = response.links.get("next", {}).get("url")
                params = {}
                first_page = False
                if nxt:
                    page_count += 1
                    if page_count >= SOFT_PAGE_LIMIT:
                        raise WindowTooLarge
                url = nxt
        except WindowTooLarge:
            for sub in reversed(strategy._window_split(window, last_value)):
                windows.appendleft(sub)


def imap_bounded(
    gate: RequestGate, tasks: Iterable[Any], fn: Callable[[Any], Any]
) -> Iterator[Any]:
    limit = 2 * gate.max_workers
    pending: set[Future[Any]] = set()
    task_iter = iter(tasks)
    exhausted = False
    try:
        while True:
            while not exhausted and len(pending) < limit:
                try:
                    task = next(task_iter)
                except StopIteration:
                    exhausted = True
                    break
                pending.add(gate.executor.submit(fn, task))
            if not pending:
                return
            done, pending = wait(pending, return_when=FIRST_COMPLETED)
            for future in done:
                yield future.result()
    finally:
        for future in pending:
            future.cancel()


class Done:
    __slots__ = ("value",)

    def __init__(self, value: Any) -> None:
        self.value = value


class _Failed:
    __slots__ = ("exc",)

    def __init__(self, exc: BaseException) -> None:
        self.exc = exc


def imap_stream(
    gate: RequestGate, tasks: Iterable[Any], fn: Callable[[Any], Iterator[Any]]
) -> Iterator[tuple[Any, Any]]:
    limit = 2 * gate.max_workers
    record_q: queue.Queue[tuple[Any, Any]] = queue.Queue(maxsize=limit)
    abort = threading.Event()

    def put(item: tuple[Any, Any]) -> bool:
        while not abort.is_set():
            try:
                record_q.put(item, timeout=0.5)
                return True
            except queue.Full:
                continue
        return False

    def drive(task: Any) -> None:
        try:
            gen = fn(task)
        except BaseException as exc:  # noqa: BLE001
            put((task, _Failed(exc)))
            return
        try:
            while True:
                try:
                    record = next(gen)
                except StopIteration as stop:
                    put((task, Done(stop.value)))
                    return
                if not put((task, record)):
                    return
        except BaseException as exc:  # noqa: BLE001
            put((task, _Failed(exc)))
        finally:
            if isinstance(gen, Generator):
                gen.close()

    pending: set[Future[None]] = set()
    task_iter = iter(tasks)
    exhausted = False
    active = 0
    try:
        while True:
            while not exhausted and active < limit:
                try:
                    task = next(task_iter)
                except StopIteration:
                    exhausted = True
                    break
                pending.add(gate.executor.submit(drive, task))
                active += 1
            if active == 0:
                return
            task, item = record_q.get()
            if isinstance(item, _Failed):
                raise item.exc
            if isinstance(item, Done):
                active -= 1
                pending = {f for f in pending if not f.done()}
                yield task, item
            else:
                yield task, item
    finally:
        abort.set()
        for future in pending:
            future.cancel()


class OrderedPrefix:
    def __init__(self) -> None:
        self._next = 0
        self._buffer: MutableMapping[int, Any] = {}

    def complete(self, seq: int, value: Any) -> Iterator[Any]:
        self._buffer[seq] = value
        while self._next in self._buffer:
            yield self._buffer.pop(self._next)
            self._next += 1
