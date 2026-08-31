from __future__ import annotations

import datetime as _dt
from typing import Any

import pytest

from insight_seed import profiles
from insight_seed.generators import base, git_repos

_TENANT = "00000000-df51-5b42-9538-d2b56b7ee953"
_ANCHOR = _dt.date(2026, 8, 11)

Emitted = dict[str, tuple[list[str], list[tuple[Any, ...]]]]

_NO_CLIENT: Any = None


@pytest.fixture(autouse=True)
def pinned_anchor(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(base, "_anchor_cache", _ANCHOR)


@pytest.fixture
def emitted(monkeypatch: pytest.MonkeyPatch) -> Emitted:
    captured: Emitted = {}

    def capture(
        client: Any, schema: str, table: str, cols: list[str], rows: list[tuple[Any, ...]]
    ) -> int:
        captured[f"{schema}.{table}"] = (list(cols), list(rows))
        return len(rows)

    monkeypatch.setattr(git_repos, "truncate", lambda client, schema, table: None)
    monkeypatch.setattr(git_repos, "bulk_insert", capture)
    return captured


def _column(emitted: Emitted, table: str, name: str) -> list[Any]:
    cols, rows = emitted[table]
    return [row[cols.index(name)] for row in rows]


def test_each_vendor_gets_its_own_bronze_relation(emitted: Emitted) -> None:
    git_repos.generate(_NO_CLIENT, profiles.build_roster("dev@company.nonpresent"), _TENANT)
    assert set(emitted) == {
        "bronze_github.repositories",
        "bronze_gitlab.projects",
        "bronze_bitbucket_cloud.repositories",
    }


@pytest.mark.parametrize(
    ("table", "column"),
    [
        ("bronze_github.repositories", "default_branch"),
        ("bronze_gitlab.projects", "default_branch"),
        ("bronze_bitbucket_cloud.repositories", "default_branch"),
    ],
)
def test_every_vendor_populates_the_default_branch(
    emitted: Emitted, table: str, column: str
) -> None:
    git_repos.generate(_NO_CLIENT, profiles.build_roster("dev@company.nonpresent"), _TENANT)
    branches = _column(emitted, table, column)
    assert branches, f"no rows for {table}"
    assert all(branches), f"should populate {column}: {branches!r}"


def test_the_unique_key_follows_the_connector_format(emitted: Emitted) -> None:
    git_repos.generate(_NO_CLIENT, profiles.build_roster("dev@company.nonpresent"), _TENANT)
    keys = _column(emitted, "bronze_github.repositories", "unique_key")
    ids = _column(emitted, "bronze_github.repositories", "id")
    assert all(k.startswith(f"{_TENANT}:") for k in keys)
    assert all(k.endswith(f":{i}") for k, i in zip(keys, ids, strict=True))


def test_the_repository_with_no_ci_is_still_seeded(emitted: Emitted) -> None:
    git_repos.generate(_NO_CLIENT, profiles.build_roster("dev@company.nonpresent"), _TENANT)
    assert "acme/legacy-archive" in _column(emitted, "bronze_github.repositories", "full_name")
