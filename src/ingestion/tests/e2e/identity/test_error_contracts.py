"""Contract: the error-path status codes the rest of the suite leaves out.

Closes the per-status-code gaps of the coverage report: validation 400s on
the mutating endpoints, 404s for unknown ids on DELETE/GET, the 401/403 gate
proven on every route (not just a sibling), malformed-UUID / query-param
400s, and the nil-tenant 400. Each case was probed against BOTH
implementations before being added; behavior only the Rust port has (see
`lib.identity.supports_strict_input_validation`) is asserted in its own
capability-gated section, so this file is the full Rust surface while the
.NET run stays green.

Deliberately absent here:
- 503 on POST /v1/persons-seed (seed queue full): the queue capacity is a
  compile-time constant (gear.rs, 100) and the refusal needs the channel
  full at the instant of the POST — not deterministically inducible from a
  black-box test, the same reason the coverage gate excludes >=500 codes
  (SERVER_FAULT_FLOOR). Pinned instead by Rust unit tests on the extracted
  refusal path (identity-resolution src/api/seed.rs, `try_enqueue_job`).

Nothing here mutates state: the 400s fail validation before any write, the
404s target ids that don't exist, and the 401/403s never pass the gate.
"""

from __future__ import annotations

import uuid

import pytest

from identity.contract import problem
from lib import identity_seed as seed

pytestmark = pytest.mark.identity

# A well-formed UUID no fixture row uses (same convention as test_subchart).
UNKNOWN_ID = "00000000-0000-4000-8000-00000000dead"

NIL_UUID = uuid.UUID(int=0)

TOO_LONG_REASON = "x" * 501


# ── POST /v1/persons-seed ─────────────────────────────────────────────────


def test_persons_seed_unsupported_mode_400(api) -> None:
    """Only 'link-by-email' exists; the refusal happens before any enqueue,
    so nothing is written."""
    r = api.post("/v1/persons-seed", json={"mode": "no-such-mode"})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


# ── GET /v1/persons-seed/{id} + list — the gate proven per-route ─────────


def test_persons_seed_get_unknown_404(api) -> None:
    r = api.get(f"/v1/persons-seed/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
    problem(r)


def test_persons_seed_get_403_non_admin(bob_api) -> None:
    r = bob_api.get(f"/v1/persons-seed/{UNKNOWN_ID}")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_persons_seed_get_401_unauthenticated(anon_api) -> None:
    assert anon_api.get(f"/v1/persons-seed/{UNKNOWN_ID}").status_code == 401


def test_persons_seed_list_403_non_admin(bob_api) -> None:
    r = bob_api.get("/v1/persons-seed")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_persons_seed_list_401_unauthenticated(anon_api) -> None:
    assert anon_api.get("/v1/persons-seed").status_code == 401


# ── /v1/roles ─────────────────────────────────────────────────────────────


def test_role_create_empty_name_400(api) -> None:
    r = api.post("/v1/roles", json={"name": ""})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_role_create_name_too_long_400(api) -> None:
    """Both validators cap the name at 64 chars."""
    r = api.post("/v1/roles", json={"name": "r" * 65})
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_role_create_401_unauthenticated(anon_api) -> None:
    assert anon_api.post("/v1/roles", json={"name": "e2e-nope"}).status_code == 401


def test_role_delete_unknown_404(api) -> None:
    r = api.delete(f"/v1/roles/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
    problem(r)


def test_role_delete_403_non_admin(bob_api) -> None:
    r = bob_api.delete(f"/v1/roles/{UNKNOWN_ID}")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_role_delete_401_unauthenticated(anon_api) -> None:
    assert anon_api.delete(f"/v1/roles/{UNKNOWN_ID}").status_code == 401


# ── /v1/person-roles ──────────────────────────────────────────────────────


def test_person_role_create_nil_person_400(api) -> None:
    r = api.post(
        "/v1/person-roles",
        json={"person_id": str(NIL_UUID), "role_id": str(seed.ADMIN_ROLE_ID)},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_person_role_create_nil_role_400(api) -> None:
    r = api.post(
        "/v1/person-roles",
        json={"person_id": str(seed.BOB), "role_id": str(NIL_UUID)},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_person_role_create_reason_too_long_400(api) -> None:
    """Both validators cap `reason` at 500 chars on the CREATE (the DELETE
    body is validated only by the Rust side — a divergence, not tested)."""
    r = api.post(
        "/v1/person-roles",
        json={
            "person_id": str(seed.BOB),
            "role_id": str(seed.ADMIN_ROLE_ID),
            "reason": TOO_LONG_REASON,
        },
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_person_role_create_401_unauthenticated(anon_api) -> None:
    r = anon_api.post(
        "/v1/person-roles",
        json={"person_id": str(seed.BOB), "role_id": str(seed.ADMIN_ROLE_ID)},
    )
    assert r.status_code == 401


def test_person_role_delete_unknown_404(api) -> None:
    r = api.delete(f"/v1/person-roles/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
    problem(r)


def test_person_role_delete_403_non_admin(bob_api) -> None:
    r = bob_api.delete(f"/v1/person-roles/{UNKNOWN_ID}")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_person_role_delete_401_unauthenticated(anon_api) -> None:
    assert anon_api.delete(f"/v1/person-roles/{UNKNOWN_ID}").status_code == 401


# ── /v1/visibility ────────────────────────────────────────────────────────


def test_visibility_create_nil_viewer_400(api) -> None:
    r = api.post(
        "/v1/visibility",
        json={"viewer_person_id": str(NIL_UUID), "viewed_person_id": str(seed.HIDDEN)},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_visibility_create_reason_too_long_400(api) -> None:
    r = api.post(
        "/v1/visibility",
        json={
            "viewer_person_id": str(seed.ALICE),
            "viewed_person_id": str(seed.HIDDEN),
            "reason": TOO_LONG_REASON,
        },
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_visibility_create_401_unauthenticated(anon_api) -> None:
    r = anon_api.post(
        "/v1/visibility",
        json={"viewer_person_id": str(seed.ALICE), "viewed_person_id": str(seed.HIDDEN)},
    )
    assert r.status_code == 401


def test_visibility_delete_unknown_404(api) -> None:
    r = api.delete(f"/v1/visibility/{UNKNOWN_ID}")
    assert r.status_code == 404, f"status={r.status_code} body={r.text}"
    problem(r)


def test_visibility_delete_403_non_admin(bob_api) -> None:
    r = bob_api.delete(f"/v1/visibility/{UNKNOWN_ID}")
    assert r.status_code == 403, f"status={r.status_code} body={r.text}"


def test_visibility_delete_401_unauthenticated(anon_api) -> None:
    assert anon_api.delete(f"/v1/visibility/{UNKNOWN_ID}").status_code == 401


# ── GET /v1/subchart (forest) — param validation proven on THIS route ────


def test_forest_negative_depth_400(api) -> None:
    r = api.get("/v1/subchart?depth=-1")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_forest_invalid_valid_at_400(api) -> None:
    r = api.get("/v1/subchart?valid_at=not-a-date")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"


# ── malformed ids / query params → 400 (route + binder level) ────────────


def test_role_delete_malformed_uuid_400(api) -> None:
    assert api.delete("/v1/roles/not-a-uuid").status_code == 400


def test_person_role_delete_malformed_uuid_400(api) -> None:
    assert api.delete("/v1/person-roles/not-a-uuid").status_code == 400


def test_visibility_delete_malformed_uuid_400(api) -> None:
    assert api.delete("/v1/visibility/not-a-uuid").status_code == 400


def test_persons_seed_get_malformed_uuid_400(api) -> None:
    assert api.get("/v1/persons-seed/not-a-uuid").status_code == 400


def test_person_roles_list_malformed_limit_400(api) -> None:
    assert api.get("/v1/person-roles?limit=abc").status_code == 400


def test_persons_seed_list_malformed_limit_400(api) -> None:
    assert api.get("/v1/persons-seed?limit=abc").status_code == 400


def test_person_roles_list_malformed_person_filter_400(api) -> None:
    assert api.get("/v1/person-roles?person=not-a-uuid").status_code == 400


def test_visibility_list_malformed_active_400(api) -> None:
    assert api.get("/v1/visibility?active=maybe").status_code == 400


# ── Rust-only strict validation (capability-gated) ────────────────────────


@pytest.fixture
def strict_api(identity_svc, api):
    """`api`, but only on an implementation with the strict input validation
    the Rust port added — skipped (before any request) elsewhere."""
    if not identity_svc.supports_strict_input_validation:
        pytest.skip("strict input validation is Rust-only (see lib.identity)")
    return api


def test_person_role_delete_reason_too_long_400(strict_api) -> None:
    """The revoke body's `reason` cap is enforced BEFORE the lookup, so an
    unknown id keeps this non-mutating."""
    r = strict_api.request(
        "DELETE", f"/v1/person-roles/{UNKNOWN_ID}", json={"reason": TOO_LONG_REASON}
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_visibility_delete_reason_too_long_400(strict_api) -> None:
    r = strict_api.request(
        "DELETE", f"/v1/visibility/{UNKNOWN_ID}", json={"reason": TOO_LONG_REASON}
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_visibility_create_nil_viewed_400(strict_api) -> None:
    """A present-but-nil target is nonsense (only an ABSENT viewed_person_id
    means whole-tree visibility); .NET creates the grant, Rust refuses."""
    r = strict_api.post(
        "/v1/visibility",
        json={"viewer_person_id": str(seed.ALICE), "viewed_person_id": str(NIL_UUID)},
    )
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
    problem(r)


def test_subchart_malformed_uuid_400(strict_api) -> None:
    """Rust rejects the unparseable path id as 400; the .NET binder answers
    404 — a reviewed divergence, so only the Rust behavior is pinned."""
    assert strict_api.get("/v1/subchart/not-a-uuid").status_code == 400


# ── nil tenant in the JWT → 400 tenant_unresolved ────────────────────────


def test_nil_tenant_400(identity_svc) -> None:
    """A token whose tenant_id claim is the nil UUID: both implementations
    refuse with an explicit 400 (tenant_unresolved) instead of silently
    querying an empty tenant."""
    with identity_svc.client(sub=str(seed.ALICE), tenant=str(NIL_UUID)) as c:
        r = c.get("/v1/roles")
    assert r.status_code == 400, f"status={r.status_code} body={r.text}"
