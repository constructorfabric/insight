"""
Shared utilities for the row generators.

* Calendar walker that yields business-day-aware dates over a window.
* Deterministic per-(person, day) RNG so re-runs reproduce.
* Persona multiplier so different ICs have different output volumes
  without needing a separate table to record it.
"""

from __future__ import annotations

import datetime as _dt
import hashlib
import logging
import os
import random
from typing import TYPE_CHECKING

from .. import config

LOG = logging.getLogger("seed.generators")

if TYPE_CHECKING:
    pass

# ─── Calendar ────────────────────────────────────────────────────────────

UTC = _dt.UTC

# Env knobs are parsed by `config`, the one reader the manifest builder uses too
# — two copies computing `now()` independently disagree across UTC midnight, and
# the manifest would then report a window the rows do not sit in. Resolved ONCE
# per process by the helpers below, and recorded in the manifest, so a run is
# reproducible from what it reports.
DEFAULT_SEED_DAYS = config.DEFAULT_SEED_DAYS

_anchor_cache: _dt.date | None = None


def anchor_date() -> _dt.date:
    """The last calendar day that carries seeded activity, inclusive.

    Every generator derives its dates from this — nothing may read the clock
    directly, or two generators in the same run could straddle UTC midnight
    and desynchronise (and a re-seed the next day would silently produce a
    different dataset).

    `SEED_ANCHOR_DATE` pins it to an ISO date; the literal `today` selects
    the default explicitly. The default is *yesterday* UTC, because the
    current day is deliberately excluded (a partial day fights the gold
    views' day-aligned aggregates).

    The default deliberately tracks the calendar rather than a committed
    constant: a fixed past anchor ages one day per day, and the metric
    surfaces this seed exists to populate are read through a UI whose
    default period is relative to now. A stand seeded against a constant
    from months ago renders empty while being perfectly "deterministic".
    Determinism here means "a given anchor always yields the same bytes",
    and the anchor is reported in the manifest — so CI and test stands pin
    it explicitly and get exact reproducibility, while the developer inner
    loop stays populated.
    """
    global _anchor_cache
    if _anchor_cache is None:
        _anchor_cache = config.parse_anchor_date(os.environ)
    return _anchor_cache


def anchor_datetime() -> _dt.datetime:
    """Anchor as an aware UTC midnight datetime.

    Aware, not naive: clickhouse-connect serialises DateTime with
    `int(x.timestamp())`, which resolves a naive value in the seeding
    process's local timezone and would make the seeding host an undeclared
    input.
    """
    return _dt.datetime.combine(anchor_date(), _dt.time(), tzinfo=UTC)


def seed_days(default: int = DEFAULT_SEED_DAYS) -> int:
    """Length of the seeded activity window, in days."""
    return config.parse_seed_days(os.environ, default)


def days_window(days: int, end: _dt.date | None = None) -> list[_dt.date]:
    """Return `days` consecutive dates ending on the anchor, inclusive.

    `end` is EXCLUSIVE and defaults to the day after `anchor_date()`, so the
    window is `[anchor - days + 1 .. anchor]`. Callers should not pass `end`
    unless they genuinely need a different window; the default is what keeps
    every generator on the same calendar.
    """
    if end is None:
        end = anchor_date() + _dt.timedelta(days=1)
    return [end - _dt.timedelta(days=i) for i in range(days, 0, -1)]


def weekday_multiplier(d: _dt.date) -> float:
    """1.0 on weekdays, 0.2 on weekends. Holidays are out of scope."""
    return 1.0 if d.weekday() < 5 else 0.2


# ─── Deterministic RNG ───────────────────────────────────────────────────


def seeded_rng(person_uuid: str, d: _dt.date, salt: str = "") -> random.Random:
    """Deterministic per-(person, day, salt) random.Random instance.

    Re-running the seed with the same inputs reproduces the same rows.
    `salt` lets different generators (git vs collab) draw independent
    sequences for the same (person, day).
    """
    key = f"{person_uuid}|{d.isoformat()}|{salt}".encode()
    digest = hashlib.blake2b(key, digest_size=16).digest()
    seed = int.from_bytes(digest, "big")
    return random.Random(seed)


def persona_multiplier(person_uuid: str) -> float:
    """Stable [0.6, 1.4] per-person scale factor."""
    digest = hashlib.blake2b(person_uuid.encode(), digest_size=8).digest()
    raw = int.from_bytes(digest, "big") / 2**64
    return 0.6 + raw * 0.8


# ─── Cell pickers ────────────────────────────────────────────────────────


def poisson(rng: random.Random, mean: float) -> int:
    """Knuth's algorithm — fine for the small means we use (≤30)."""
    if mean <= 0:
        return 0
    cutoff = 2.718281828 ** (-mean)
    k, p = 0, 1.0
    while True:
        k += 1
        p *= rng.random()
        if p < cutoff:
            return k - 1


def clamp(value: float, lo: float, hi: float) -> float:
    return max(lo, min(hi, value))


def deterministic_uuid(*parts: str) -> str:
    """Build a UUID-shaped hex string from input parts. Stable across runs."""
    digest = hashlib.blake2b("|".join(parts).encode(), digest_size=16).hexdigest()
    return f"{digest[:8]}-{digest[8:12]}-{digest[12:16]}-{digest[16:20]}-{digest[20:]}"


def deterministic_int(*parts: str) -> int:
    """Build a stable positive Int64 from input parts. For integer id columns."""
    digest = hashlib.blake2b("|".join(parts).encode(), digest_size=8).digest()
    return int.from_bytes(digest, "big") & 0x7FFFFFFFFFFFFFFF
