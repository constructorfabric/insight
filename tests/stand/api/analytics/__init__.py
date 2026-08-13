"""Contract tests for the analytics service, one module per path group.

Reached at `/api/analytics/*` through the gateway. Fixtures, the scratch
policy and the response models are the api/ package's, one level up — a
resource created here is deleted by the same session-scoped leak sweep that
covers identity.
"""

from __future__ import annotations

from collections.abc import Sequence
from datetime import date, timedelta
from typing import Final

from insight_stand import Manifest

#: The widest `period` the analytics API will answer, in days — one below
#: `MAX_PERIOD_DAYS` (validation.rs), which rejects anything ≥400. See
#: INFRA.md, "730-day seed window". Every request wanting "a real period"
#: should go through `query_window`, not read `manifest.data_window` raw.
MAX_QUERY_SPAN_DAYS: Final[int] = 399


def query_window(manifest: Manifest, *, max_days: int = MAX_QUERY_SPAN_DAYS) -> tuple[str, str]:
    """The widest slice of the seeded window the analytics API will actually answer for.

    Returns `(from, to)` as ISO dates: the manifest's `data_window` end, and a
    start no more than `max_days` before it. A seed window narrower than the
    cap is returned unchanged, so the ordinary case still asks about everything
    that was seeded.

    The TAIL rather than the head, deliberately. Both ends carry seeded rows,
    so either would answer — but the recent end is the one a dashboard opens
    on, and it is the end that keeps answering as the seed's anchor moves.
    Anchoring to the start would slowly walk the query off the data as the
    window grows.

    Raises `AssertionError` rather than returning something plausible when the
    manifest's window is unreadable: a malformed `data_window` means the seed
    manifest is not what this suite thinks it is, and every date arithmetic
    below would otherwise invent a period out of a parse failure.
    """
    start_text, _, end_text = manifest.data_window.partition("..")
    assert start_text and end_text, (
        f"the manifest at {manifest.source_path} carries data_window "
        f"{manifest.data_window!r}, which is not a `from..to` range — there is no period "
        f"to ask about."
    )
    try:
        start = date.fromisoformat(start_text)
        end = date.fromisoformat(end_text)
    except ValueError as exc:
        raise AssertionError(
            f"the manifest at {manifest.source_path} carries data_window "
            f"{manifest.data_window!r}, whose ends are not ISO dates: {exc}"
        ) from None

    earliest = end - timedelta(days=max_days)
    return max(start, earliest).isoformat(), end.isoformat()


__all__: Sequence[str] = ("MAX_QUERY_SPAN_DAYS", "query_window")
