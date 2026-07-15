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
import random
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    import clickhouse_connect.driver.client

# ─── Calendar ────────────────────────────────────────────────────────────

UTC = _dt.UTC


def days_window(days: int, end: _dt.date | None = None) -> list[_dt.date]:
    """Return `days` consecutive dates ending the day BEFORE `end`.

    `end` defaults to "today (UTC)". The current day is excluded so
    partial-day rows don't fight the gold views' day-aligned aggregates.
    """
    if end is None:
        end = _dt.datetime.now(UTC).date()
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


# ─── Insert helpers ──────────────────────────────────────────────────────


def truncate(
    client: clickhouse_connect.driver.client.Client,
    schema: str,
    table: str,
) -> None:
    """Idempotent reset: TRUNCATE before INSERT."""
    client.command(f"TRUNCATE TABLE IF EXISTS `{schema}`.`{table}`")


def bulk_insert(
    client: clickhouse_connect.driver.client.Client,
    schema: str,
    table: str,
    columns: list[str],
    rows: list[tuple[object, ...]],
) -> int:
    """Insert `rows` and return the count. No-op on empty input."""
    if not rows:
        return 0
    client.insert(table, rows, column_names=columns, database=schema)
    return len(rows)
