from __future__ import annotations

import datetime as _dt
import hashlib
from typing import Any

import pytest

from insight_seed import profiles
from insight_seed.generators import base, git

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 8, 11)
_DAYS = 60

Rows = dict[str, list[tuple[Any, ...]]]


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


def test_the_commit_rows_are_unchanged_by_the_extraction(emitted: Rows) -> None:
    """Locked signature of the current draws. The history builder owns the RNG
    after this task; if its draw ORDER diverges, every seeded stand's data
    changes silently and this digest moves."""
    roster = profiles.build_roster("dev@company.nonpresent")
    git.seed_class_git_commits(None, roster, _TENANT, _DAYS)  # type: ignore[arg-type]
    rows = emitted["silver.class_git_commits"]

    digest = hashlib.blake2b(repr(rows).encode(), digest_size=16).hexdigest()
    assert len(rows) == 2022, f"row count moved: {len(rows)}"
    assert digest == "b0636eaf3ccd06719bb677df751b8668"


def test_the_pull_request_rows_are_unchanged_by_the_extraction(emitted: Rows) -> None:
    roster = profiles.build_roster("dev@company.nonpresent")
    git.seed_class_git_pull_requests(None, roster, _TENANT, _DAYS)  # type: ignore[arg-type]
    rows = emitted["silver.class_git_pull_requests"]

    digest = hashlib.blake2b(repr(rows).encode(), digest_size=16).hexdigest()
    assert len(rows) == 321, f"row count moved: {len(rows)}"
    assert digest == "d5102560b99ef0b37e0e5bde59ce412b"
