"""render_contract.py — the documented transform from an API metric value to the
string a dashboard tile must display.

Gold has two contracts: the API value, and the UI render. This module is the
*render* contract — `displayed == documented_transform(api_value)` — written as a
pure function so it can be (a) unit-tested deterministically and (b) used by the
live e2e to assert the real DOM matches it. The frontend lives in a separate repo;
this is the shared, executable definition both sides agree to.

Rules (derived from the metric catalog `format`/`unit` fields and the ComingSoon
convention, not from the current FE implementation — so a buggy FE fails the test
rather than the test rubber-stamping the bug):

  - not-ingested metric (catalog says "ComingSoon until … ingestion lands")
        → "ComingSoon", regardless of the numeric value. NEVER a concrete 0.
  - value is None / null  → NO_DATA ("—"). NEVER "0" or "0%".
  - format 'integer'      → round half-up to whole number.
  - format 'percent' / unit '%' → round half-up to whole number, suffixed "%".
  - other numeric          → value with its unit, separated by a SPACE
                             ("0 tasks", not "0tasks"; "0 h", not "0h").

Rounding is half-up (4.4→4, 4.6→5, 98.8→99) — banker's rounding is wrong for a
user-facing number.
"""
from __future__ import annotations

import math
from typing import Optional

NO_DATA = "—"
COMING_SOON = "ComingSoon"


def round_half_up(value: float) -> int:
    """Round to nearest integer, halves away from zero (4.5→5, -4.5→-5, 4.4→4)."""
    return int(math.floor(value + 0.5)) if value >= 0 else int(math.ceil(value - 0.5))


def display_value(
    api_value: Optional[float],
    *,
    fmt: Optional[str] = None,
    unit: Optional[str] = None,
    ingested: bool = True,
) -> str:
    """Return the exact string a tile must show for this API value.

    `ingested=False` marks a metric whose source isn't wired yet (catalog
    ComingSoon). `fmt` is the catalog `format` ('integer' | 'percent' | 'hours' | …);
    `unit` is the catalog `unit` ('tasks', 'h', '%', …).
    """
    if not ingested:
        return COMING_SOON
    if api_value is None:
        return NO_DATA

    if fmt == "percent" or unit == "%":
        return f"{round_half_up(api_value)}%"
    if fmt == "integer":
        return str(round_half_up(api_value))

    # generic numeric: keep the value, space-separate the unit
    num = f"{round_half_up(api_value)}" if float(api_value).is_integer() else f"{api_value:g}"
    return f"{num} {unit}" if unit else num
