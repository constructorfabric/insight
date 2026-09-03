"""The vocabulary a recorded sync outcome may carry.

The mover's own words are stored verbatim. Nothing is translated on the way
in: a surface that reports someone else's account should not paraphrase it,
and a translation table is one more place for a meaning to be lost. What the
boundary does instead is close the set — a word outside the mover's documented
vocabulary is stored as `UNKNOWN`, so no reader ever holds a value it cannot
interpret.
"""

from __future__ import annotations

#: Every status the mover's job listing documents.
MOVER_STATUSES = frozenset(
    {"pending", "running", "incomplete", "succeeded", "failed", "cancelled"}
)

#: What a word outside `MOVER_STATUSES` becomes. Not a failure and not a
#: success — a state the reader could not read.
UNKNOWN = "unknown"

#: A job carrying one of these will not change again, so the ledger already
#: holds its last word.
#:
#: INVARIANT: `UNKNOWN` is deliberately absent. Coverage fails closed — a
#: status we could not read is one we keep re-reading until it becomes one we
#: can, at the cost of a duplicate row that resolves to the same answer.
TERMINAL_STATUSES = frozenset({"succeeded", "failed", "cancelled", "incomplete"})

#: A job carrying one of these has not finished, so nothing it reports about
#: itself is a completed measurement.
#:
#: INVARIANT: `UNKNOWN` is absent here too, and for the opposite reason to its
#: absence above. A word we could not read may well be a finished job, and
#: treating it as in flight would discard the measurement it carries.
IN_FLIGHT_STATUSES = frozenset({"pending", "running"})


def normalise(raw: object) -> str:
    """Map the mover's reported status onto the closed set."""
    if not isinstance(raw, str):
        return UNKNOWN
    word = raw.strip().lower()
    return word if word in MOVER_STATUSES else UNKNOWN


def is_terminal(status: str) -> bool:
    """Whether a recorded status closes its job for coverage purposes."""
    return status in TERMINAL_STATUSES


def is_in_flight(status: str) -> bool:
    """Whether a recorded status describes a job that has not finished."""
    return status in IN_FLIGHT_STATUSES
