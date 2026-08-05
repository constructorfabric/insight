"""Polling helpers for state that becomes true rather than being true.

A deployed stand is eventually consistent in places — a refreshable
materialised view catches up, a service finishes warming. These helpers make
that wait explicit and, crucially, LOUD on failure: they never return a boolean
a caller can forget to check, and their timeout message says what was being
waited for, so a CI log shows the actual unmet condition.

No `time.sleep()` scattered through tests. A fixed sleep is a guess that is
either too short (flaky) or too long (slow); a bounded poll is neither.
"""

from __future__ import annotations

import time
from collections.abc import Callable, Sequence


def wait_until(
    predicate: Callable[[], bool],
    *,
    timeout_s: float,
    interval_s: float = 0.5,
    description: str,
) -> None:
    """Poll `predicate` until it returns True, or raise `TimeoutError`.

    Never returns False — the only non-exceptional outcome is success, so a
    caller cannot accidentally continue on an unmet condition.
    """
    deadline = time.monotonic() + timeout_s
    attempts = 0
    while True:
        attempts += 1
        if predicate():
            return
        if time.monotonic() >= deadline:
            raise TimeoutError(
                f"timed out after {timeout_s:g}s ({attempts} attempts) waiting for: {description}"
            )
        time.sleep(interval_s)


def wait_for[T](
    supplier: Callable[[], T | None],
    *,
    timeout_s: float,
    interval_s: float = 0.5,
    description: str,
) -> T:
    """`wait_until` for a value: poll until `supplier` returns non-None.

    Saves the "poll, then fetch again" double call that `wait_until` forces
    when the thing being waited for is also the thing being used.
    """
    deadline = time.monotonic() + timeout_s
    attempts = 0
    while True:
        attempts += 1
        value = supplier()
        if value is not None:
            return value
        if time.monotonic() >= deadline:
            raise TimeoutError(
                f"timed out after {timeout_s:g}s ({attempts} attempts) waiting for: {description}"
            )
        time.sleep(interval_s)


__all__: Sequence[str] = ("wait_for", "wait_until")
