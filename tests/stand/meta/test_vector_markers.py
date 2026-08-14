"""Every stand api/ui test carries exactly one quality-vector marker.

Exactly one is what makes `-m <vector>` selection sound: pytest markers are
additive, so the earlier module-default-plus-override scheme left BOTH vectors
on an overridden test and `-m reliability` / `-m security` selections
overlapped (PR #2414 review). `tests/stand/conftest.py` rejects that shape at
collection — over the FULL collected tree, even under `-m` — so the gate is
the enforcement. This meta test is the regression net for a conftest refactor
that drops or weakens the gate: it re-checks the invariant from inside a
running session with the same `insight_stand.vectors` helpers the gate uses,
so the two cannot drift apart in what they count (distinct vector names — a
redundant same-vector re-mark is sound, two different vectors are not).

The net has limits it states rather than hides: it sees only this session's
selected items, so it SKIPS when none of them are api/ui tests instead of
passing vacuously, and it is marked `reliability` (test rigor is a
reliability signal) so a vector-sharded run keeps it in the reliability lane.
"""

from __future__ import annotations

import pytest
from insight_stand import distinct_vectors, governs_vector, quality_vectors

pytestmark = pytest.mark.reliability


def test_every_api_and_ui_item_carries_exactly_one_vector(
    request: pytest.FixtureRequest,
) -> None:
    vectors = quality_vectors(request.config.getini("markers"))
    assert vectors, "no quality-vector marker declarations found in tests/pyproject.toml"

    governed = [item for item in request.session.items if governs_vector(item.path)]
    if not governed:
        pytest.skip("no stand api/ui items in this session — nothing to certify")

    offenders = {
        item.nodeid: sorted(named)
        for item in governed
        if len(named := distinct_vectors((m.name for m in item.iter_markers()), vectors)) != 1
    }
    assert not offenders, (
        f"items without exactly one vector marker (so -m selections overlap or miss): {offenders}"
    )
