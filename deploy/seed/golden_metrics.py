"""Golden metrics — the ONLY source for the manifest's `golden_metrics[]`.

Entries here are added BY HAND from a measured, reproducible inventory. This
module deliberately contains no derivation logic: an expected value MUST NOT
be inferred from `TEAM_PROFILES` weights, generator row counts, or any other
proxy. The consuming test suite asserts every entry as an exact match and has
no other source of expectations, so a guessed value becomes a confident false
failure that reads as a product bug rather than as missing input.

An empty list is a legitimate, and currently correct, state: it costs the
suite its metric assertions (a visible gap) instead of giving it wrong ones.

WHY EMPTY TODAY
---------------
No measured inventory records an exact expected value for any metric key.
The one inventory that exists was taken against a stack whose gold build was
failing, and was explicitly marked provisional and non-reproducible once the
seed was repaired.

Two further constraints bound what may ever be added:

* Metrics computed relative to wall clock cannot be golden. The gold layer
  derives `stale_in_progress` with `metric_date = today()` and a
  `dateDiff(..., now()) > 14` predicate, so its value saturates as real time
  advances past the anchor, no matter what the seed wrote. Anything
  downstream of `insight.task_status_spans` inherits the same property via
  its `now()` interval clamp.

* An expectation must be computable from seed inputs alone AND reliably equal
  to what the gold SQL returns. Metrics whose gold model applies FINAL dedup,
  multi-table joins or window functions are not reproducible from the
  generation loop and must not be listed.

HOW TO ADD ONE
--------------
1. Seed a stand with a PINNED anchor (`SEED_ANCHOR_DATE`), so the window is
   reproducible.
2. Compute the expectation from the generator's own logic, not by reading the
   value back out of gold — reading it back makes the test assert that the
   database equals itself.
3. Confirm it is stable across a teardown-and-reseed at the same anchor.
4. Add the entry with the scope and window expressed against the anchor.
"""

from __future__ import annotations

from typing import Any

# Each entry: {metric_key, expected, scope, window, derivation: "constructed"}.
GOLDEN_METRICS: list[dict[str, Any]] = []

# Recorded in the manifest so a consumer can tell "none measured yet" apart
# from "measured and genuinely zero".
GOLDEN_METRICS_NOTE = (
    "empty: no measured inventory records an exact expected value; "
    "see deploy/seed/golden_metrics.py for the criteria to add one"
)
