"""How a stand recognises rows a test run created, rather than the seed.

A stand persists between runs and is reset by volume teardown, never by
TRUNCATE — so anything a test writes and cannot delete has to be identifiable
on sight instead. Two things make it so: a name carrying a fixed prefix and
this run's own token, and a fixed connector instance every correction is filed
under.

That matters most for the correction journal, which is append-only: a decision
written under a SEEDED connector is indistinguishable from real identity data,
and the seed preflight refuses to seed a stand that carries any — so the next
`up` fails until the volumes are torn down. Filing under the pair below is
what keeps a correction exempt.

Lives here, in the package both suites import, because it describes the STAND
rather than either suite: `tests/stand/api/scratch.py` re-exports it with the
sweep that only the API suite can run, and the UI journeys take it directly.
"""

from __future__ import annotations

import uuid
from typing import Final

#: Marks every row the suites create, so a leak is identifiable on sight.
#: INVARIANT: must match insight_seed.config.STAND_SCRATCH_PREFIX — the seed
#: preflight uses it to recognise leftover journal rows.
SCRATCH_PREFIX: Final[str] = "stand-scratch"

#: The connector instance every correction a test writes is filed under — the
#: other half of what makes an undeletable journal row recognisable. Fixed, not
#: random: a stable pair keeps those rows attributable.
#: INVARIANT: must match insight_seed.config.STAND_SCRATCH_SOURCE_TYPE, and the
#: literal `github` segment in `operations.py`'s account-read template — the
#: coverage gate folds a recorded path onto a template by exact segment
#: equality, so a divergence here makes that operation read as never exercised.
SCRATCH_SOURCE_TYPE: Final[str] = "github"

#: INVARIANT: must match insight_seed.config.STAND_SCRATCH_SOURCE_ID. Unlike the
#: type, this segment is a `{source_id}` in the coverage template, so it folds
#: whatever its value.
SCRATCH_SOURCE_ID: Final[str] = "01900000-0000-7000-8000-00000000feed"

#: One token per session: a leak becomes attributable to the run that made it.
RUN_TAG: Final[str] = uuid.uuid4().hex[:8]

#: Every name this session issued. The API suite's sweep reads it; formatting a
#: name by hand would satisfy the prefix rule and silently blind that sweep,
#: which is why `scratch_name` is the only way to make one.
_ISSUED: set[str] = set()


def scratch_name(tag: str) -> str:
    """A unique, greppable, attributable name — and register it for the sweep."""
    name = f"{SCRATCH_PREFIX}-{RUN_TAG}-{tag}-{uuid.uuid4().hex[:8]}"
    _ISSUED.add(name)
    return name


def issued_names() -> frozenset[str]:
    return frozenset(_ISSUED)
