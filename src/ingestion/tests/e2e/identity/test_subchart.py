"""Contract: GET /v1/subchart/{person_id} — the recursive org subtree."""

from __future__ import annotations

import pytest

from lib import identity_seed as seed

pytestmark = pytest.mark.identity


def _person_ids(node: dict) -> set[str]:
    """Flatten every person_id in a subchart tree, whatever the nesting key."""
    found: set[str] = set()

    def walk(value) -> None:
        if isinstance(value, dict):
            pid = value.get("person_id")
            if pid:
                found.add(pid)
            for v in value.values():
                walk(v)
        elif isinstance(value, list):
            for v in value:
                walk(v)

    walk(node)
    return found


def test_subchart_200_own_subtree(api) -> None:
    """alice's subtree covers bob → carol and the dup pair."""
    r = api.get(f"/v1/subchart/{seed.ALICE}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    ids = _person_ids(r.json())
    assert {str(seed.ALICE), str(seed.BOB), str(seed.CAROL)} <= ids, ids
    assert str(seed.HIDDEN) not in ids, ids


def test_subchart_of_subordinate_200(api) -> None:
    """A subtree rooted below the caller is visible too (bob is in alice's tree)."""
    r = api.get(f"/v1/subchart/{seed.BOB}")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    ids = _person_ids(r.json())
    assert str(seed.CAROL) in ids, ids


def test_subchart_hidden_root_denied(api) -> None:
    """A root outside the caller's visible set never renders — deny-as-404
    (a hidden person must be indistinguishable from a missing one)."""
    r = api.get(f"/v1/subchart/{seed.HIDDEN}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_subchart_unknown_person_404(api) -> None:
    r = api.get("/v1/subchart/00000000-0000-4000-8000-00000000dead")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"


def test_subchart_401_unauthenticated(anon_api) -> None:
    assert anon_api.get(f"/v1/subchart/{seed.ALICE}").status_code == 401


def test_subchart_self_200(api) -> None:
    """GET /v1/subchart (no person_id) roots the forest at the caller."""
    r = api.get("/v1/subchart")
    assert r.status_code == 200, f"status={r.status_code} body={r.text}"
    ids = _person_ids(r.json())
    assert {str(seed.ALICE), str(seed.BOB), str(seed.CAROL)} <= ids, ids


def test_subchart_self_401_unauthenticated(anon_api) -> None:
    assert anon_api.get("/v1/subchart").status_code == 401
