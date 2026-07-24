"""Contract: the query-parameter surface of the list/tree endpoints.

Filters (person / role / viewer / viewed / active), paging (limit), the
subchart lenses (depth, valid_at) and their validation errors — the parts a
consumer actually parameterizes, so the replacement must honor them too.
"""

from __future__ import annotations

import pytest

from identity.contract import list_response, problem
from lib import identity_seed as seed

pytestmark = pytest.mark.identity


def _rows(api, path: str) -> list[dict]:
    """GET a list endpoint: status first, then the strict wire envelope."""
    r = api.get(path)
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    rows, _cursor = list_response(r.json())
    return rows


# ── /v1/person-roles filters ─────────────────────────────────────────────


def test_person_roles_filter_by_person(api) -> None:
    rows = _rows(api, f"/v1/person-roles?person={seed.ALICE}")
    assert rows, "alice holds the seeded admin assignment"
    assert all(row["person_id"] == str(seed.ALICE) for row in rows), rows

    rows = _rows(api, f"/v1/person-roles?person={seed.CAROL}")
    assert rows == [], rows


def test_person_roles_filter_by_role_and_active(api) -> None:
    rows = _rows(api, f"/v1/person-roles?role={seed.ADMIN_ROLE_ID}&active=true")
    assert any(row["person_id"] == str(seed.ALICE) for row in rows), rows
    assert all(row["role_id"] == str(seed.ADMIN_ROLE_ID) for row in rows), rows
    assert all(row.get("valid_to") is None for row in rows), rows


def test_person_roles_limit(api) -> None:
    rows = _rows(api, "/v1/person-roles?limit=1")
    assert len(rows) == 1, rows


# ── /v1/visibility filters ───────────────────────────────────────────────


def test_visibility_filter_by_viewer(api) -> None:
    rows = _rows(api, f"/v1/visibility?viewer={seed.BOB}")
    assert any(row["viewed_person_id"] == str(seed.HIDDEN) for row in rows), rows

    rows = _rows(api, f"/v1/visibility?viewer={seed.CAROL}")
    assert rows == [], rows


def test_visibility_filter_by_viewed_and_active(api) -> None:
    rows = _rows(api, f"/v1/visibility?viewed={seed.HIDDEN}&active=true")
    assert any(row["viewer_person_id"] == str(seed.BOB) for row in rows), rows
    assert all(row.get("valid_to") is None for row in rows), rows


# ── /v1/subchart lenses ──────────────────────────────────────────────────


def test_subchart_depth_zero_is_root_only(api) -> None:
    """depth=0 cuts the descent at the root — no subordinates rendered."""
    r = api.get(f"/v1/subchart/{seed.ALICE}?depth=0")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert str(seed.BOB) not in r.text, r.text


def test_subchart_depth_one_excludes_grandchildren(api) -> None:
    r = api.get(f"/v1/subchart/{seed.ALICE}?depth=1")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    assert str(seed.BOB) in r.text, r.text
    assert str(seed.CAROL) not in r.text, r.text


def test_subchart_negative_depth_400(api) -> None:
    r = api.get(f"/v1/subchart/{seed.ALICE}?depth=-1")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_subchart_invalid_valid_at_400(api) -> None:
    r = api.get(f"/v1/subchart/{seed.ALICE}?valid_at=not-a-date")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


def test_subchart_valid_at_before_seed_renders_bare_root(api) -> None:
    """A point-in-time lens BEFORE the fixture rows' valid_from: the contract
    (pinned from the live .NET service) is 200 with the root rendered bare —
    person_id present, no observation-derived fields, no subordinates. NOT a
    404 and NOT an absent root; a divergence here is consumer-visible.
    (persons-seed list filters live in test_persons_seed.py — they need the
    ClickHouse fixture to create real operations.)"""
    r = api.get(f"/v1/subchart/{seed.ALICE}?valid_at=2000-01-01T00:00:00Z")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    root = r.json()["root"]
    assert root["person_id"] == str(seed.ALICE), root
    assert root["subordinates"] == [], root
    assert root.get("email") is None, root
