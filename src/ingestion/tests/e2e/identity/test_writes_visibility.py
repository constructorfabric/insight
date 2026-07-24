"""Contract: the mutating visibility endpoints — grant / revoke, with the
behavioral proof that a grant actually changes what /v1/profiles resolves."""

from __future__ import annotations

import pytest

from lib import identity_seed as seed

pytestmark = [pytest.mark.identity, pytest.mark.mutating]


def _resolve_hidden(client):
    return client.post("/v1/profiles", json={"value_type": "email", "value": seed.HIDDEN_EMAIL})


def test_visibility_grant_changes_resolution_then_revoke_restores(api) -> None:
    """alice cannot see `hidden` (404) → grant → 200 → revoke → 404 again.

    This is the end-to-end proof the grant feeds the read path's visibility
    CTE, not just a row in a table.
    """
    assert _resolve_hidden(api).status_code == 404

    g = api.post(
        "/v1/visibility",
        json={"viewer_person_id": str(seed.ALICE), "viewed_person_id": str(seed.HIDDEN)},
    )
    assert g.status_code == 201, f"status={g.status_code} body={g.text}"
    grant_id = g.json()["visibility_id"]

    try:
        r = _resolve_hidden(api)
        assert r.status_code == 200, f"status={r.status_code} body={r.text}"
        assert r.json()["person_id"] == str(seed.HIDDEN)
    finally:
        d = api.delete(f"/v1/visibility/{grant_id}")
        assert d.status_code == 204, f"status={d.status_code} body={d.text}"

    assert _resolve_hidden(api).status_code == 404


def test_visibility_grant_403_non_admin(bob_api) -> None:
    r = bob_api.post(
        "/v1/visibility",
        json={"viewer_person_id": str(seed.BOB), "viewed_person_id": str(seed.ALICE)},
    )
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"
