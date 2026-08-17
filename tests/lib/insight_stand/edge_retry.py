"""Retrying the answers that come from in front of the stand, not from it.

A stand reached through a CDN or a reverse proxy can be answered by that
intermediary in its own right: 520-527 are the range one uses to say the origin
was unreachable, timed out, dropped the connection, or handed back something it
could not parse. None of those is the product's answer, and a suite that records
one has measured the path rather than the thing under test.

Retrying is therefore not leniency about product faults — 5xx codes the product
itself emits (500, 502, 503) are never retried here, and a rate-limited 429 is a
real answer a test may be asserting on. Only the intermediary's own range is.

The range splits on what it tells you about the origin:

* **521-527** say the origin never answered — refused the connection, timed
  out, failed the TLS handshake. Nothing ran, so any request may be repeated.
* **520** says the origin answered and the answer was unusable. It may have run
  first, so repeating it can run it twice.

That is why the caller declares its own repeatability rather than having it
inferred from the HTTP method: a `GET` that spends a one-time authorization code
is no safer to repeat than a `POST`, and the split is about effects.

Retries are counted so a run can say how much of its green came from a second
attempt. A suite that silently retries its way to a pass hides exactly the edge
instability someone should be looking at.
"""

from __future__ import annotations

import time
from collections import Counter
from collections.abc import Callable, Mapping, Sequence

#: What an intermediary answers about an origin it could not get a response from.
NEVER_ANSWERED: frozenset[int] = frozenset({521, 522, 523, 524, 525, 526, 527})

#: 520 as well: the origin answered, unusably, and may have run the request.
ANY_EDGE_ERROR: frozenset[int] = NEVER_ANSWERED | {520}

MAX_ATTEMPTS = 4
BACKOFF_BASE_S = 1.0

_retried: Counter[int] = Counter()


def record(status_code: int) -> None:
    _retried[status_code] += 1


def reset() -> None:
    _retried.clear()


def retried() -> Mapping[int, int]:
    """How many retries each edge status caused, for the run's summary."""
    return dict(_retried)


def backoff_s(attempt: int) -> float:
    """Seconds to wait before attempt `attempt` (1-based): 1s, 2s, 4s."""
    return BACKOFF_BASE_S * 2 ** (attempt - 1)


def send[T](
    attempt: Callable[[], T],
    status_of: Callable[[T], int],
    *,
    repeatable_on: frozenset[int] = NEVER_ANSWERED,
    sleep: Callable[[float], None] = time.sleep,
) -> T:
    """Call `attempt` until it answers with something the stand actually said.

    `repeatable_on` is the set of edge statuses this particular call may be
    repeated after — `ANY_EDGE_ERROR` for a request with no effect, the default
    `NEVER_ANSWERED` for one that must not run twice.

    The last response is returned as-is when the attempts run out, so a
    persistent edge fault fails the test that asked for it, with the edge's own
    status, rather than being converted into an exception from here.
    """
    for number in range(1, MAX_ATTEMPTS + 1):
        result = attempt()
        status = status_of(result)
        if status not in repeatable_on or number == MAX_ATTEMPTS:
            return result
        record(status)
        sleep(backoff_s(number))
    raise AssertionError("unreachable: the loop returns on its last attempt")


__all__: Sequence[str] = (
    "ANY_EDGE_ERROR",
    "BACKOFF_BASE_S",
    "MAX_ATTEMPTS",
    "NEVER_ANSWERED",
    "backoff_s",
    "record",
    "reset",
    "retried",
    "send",
)
