"""Contract: the query-parameter surface of the list/tree endpoints.

Filters (person / role / viewer / viewed / active), paging (limit), the
subchart lenses (depth, valid_at) and their validation errors — the parts a
consumer actually parameterizes, so the replacement must honor them too.
"""

from __future__ import annotations

import pytest

from identity.contract import items_of, problem
from lib import identity_seed as seed

pytestmark = pytest.mark.identity


# ── /v1/person-roles filters ─────────────────────────────────────────────


def test_person_roles_filter_by_person(api) -> None:
    rows = items_of(api.get(f"/v1/person-roles?person={seed.ALICE}").json())
    assert rows, "alice holds the seeded admin assignment"
    assert all(row["person_id"] == str(seed.ALICE) for row in rows), rows

    rows = items_of(api.get(f"/v1/person-roles?person={seed.CAROL}").json())
    assert rows == [], rows


def test_person_roles_filter_by_role_and_active(api) -> None:
    rows = items_of(
        api.get(f"/v1/person-roles?role={seed.ADMIN_ROLE_ID}&active=true").json()
    )
    assert any(row["person_id"] == str(seed.ALICE) for row in rows), rows
    assert all(row["role_id"] == str(seed.ADMIN_ROLE_ID) for row in rows), rows
    assert all(row.get("valid_to") is None for row in rows), rows


def test_person_roles_limit(api) -> None:
    rows = items_of(api.get("/v1/person-roles?limit=1").json())
    assert len(rows) == 1, rows


# ── /v1/visibility filters ───────────────────────────────────────────────


def test_visibility_filter_by_viewer(api) -> None:
    rows = items_of(api.get(f"/v1/visibility?viewer={seed.BOB}").json())
    assert any(row["viewed_person_id"] == str(seed.HIDDEN) for row in rows), rows

    rows = items_of(api.get(f"/v1/visibility?viewer={seed.CAROL}").json())
    assert rows == [], rows


def test_visibility_filter_by_viewed_and_active(api) -> None:
    rows = items_of(api.get(f"/v1/visibility?viewed={seed.HIDDEN}&active=true").json())
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


def test_subchart_valid_at_before_seed_is_empty(api) -> None:
    """A point-in-time lens BEFORE the fixture rows' valid_from sees no tree —
    the seeded edges did not exist yet."""
    r = api.get(f"/v1/subchart/{seed.ALICE}?valid_at=2000-01-01T00:00:00Z")
    assert r.status_code in {200, 404}, f"status={r.status_code} body={r.text}"
    if r.status_code == 200:
        assert str(seed.BOB) not in r.text, r.text


# ── /v1/persons-seed list filters ────────────────────────────────────────


def test_persons_seed_list_limit(seed_ops_api) -> None:
    rows = items_of(seed_ops_api.get("/v1/persons-seed?limit=1").json())
    assert len(rows) <= 1, rows


def test_persons_seed_list_status_filter(seed_ops_api) -> None:
    rows = items_of(seed_ops_api.get("/v1/persons-seed?status=queued").json())
    assert all(row["status"] == "queued" for row in rows), rows
