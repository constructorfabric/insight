from __future__ import annotations

import datetime as _dt
import hashlib
from typing import Any

import pytest

from insight_seed import profiles
from insight_seed.generators import base, git
from insight_seed.generators.git_history import build_history

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 8, 11)
_DAYS = 60

Rows = dict[str, list[tuple[Any, ...]]]

COMMIT_DIGEST = "25bdaad252c3a4843beaa323553c4fba"
PULL_REQUEST_DIGEST = "6b92cfbd3c93a79b5ac8a44db9d2950f"


@pytest.fixture(autouse=True)
def pinned_anchor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(base, "_anchor_cache", _ANCHOR)


@pytest.fixture
def emitted(monkeypatch: pytest.MonkeyPatch) -> Rows:
    captured: Rows = {}

    def capture(
        client: Any, schema: str, table: str, cols: list[str], rows: list[tuple[Any, ...]]
    ) -> int:
        captured[f"{schema}.{table}"] = list(rows)
        return len(rows)

    monkeypatch.setattr(git, "truncate", lambda client, schema, table: None)
    monkeypatch.setattr(git, "bulk_insert", capture)
    return captured


def test_the_commit_draws_are_unchanged(emitted: Rows) -> None:
    """Locked signature of the current draws. The history builder owns the RNG,
    and if its draw ORDER diverges every seeded stand's data changes silently —
    so the digest is taken over the history itself rather than over whatever
    shape the generator happens to write it in."""
    roster = profiles.build_roster("dev@company.nonpresent")

    history = build_history(roster, _DAYS)

    commits = [(day.person.email, day.date, commit) for day in history for commit in day.commits]
    digest = hashlib.blake2b(repr(commits).encode(), digest_size=16).hexdigest()
    assert len(commits) == 2022, f"commit count moved: {len(commits)}"
    assert digest == COMMIT_DIGEST


def test_the_pull_request_draws_are_unchanged(emitted: Rows) -> None:
    roster = profiles.build_roster("dev@company.nonpresent")

    history = build_history(roster, _DAYS)

    prs = [(day.person.email, day.date, pr) for day in history for pr in day.prs]
    digest = hashlib.blake2b(repr(prs).encode(), digest_size=16).hexdigest()
    assert len(prs) == 321, f"pull request count moved: {len(prs)}"
    assert digest == PULL_REQUEST_DIGEST
