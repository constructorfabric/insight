"""`GET /v1/me` — the caller's identity and active roles, as the admin gate sees them.

    GET /v1/me   200 admin operator · 200 empty for a lead · 200 empty for the realm admin

The SPA gates its admin surfaces on this answer, so what is worth proving on the
deployed path is exactly the two confusions the endpoint exists to prevent:

* the answer comes from `identity.person_roles`, not from the login token — the
  CEO's `insight-admin` REALM role must NOT appear here, or the frontend would
  show an admin console to someone every admin endpoint refuses with 403;
* an empty list is the well-formed "not an admin" answer — a caller without the
  row gets 200 and `roles: []`, never a refusal, so the SPA needs no 403 probe.

The admin-operator case is the same person `test_admin.py` proves CAN open the
gated routes; here the endpoint must SAY so, with the seeded `admin` role under
its fixed id. The 401 half is in `test_gateway.py`, swept over every operation.
"""

from __future__ import annotations

import pytest
from insight_stand import ADMIN_ROLE, ApiClient, PersonaSession, identity_path

from ..schemas import MeResponse
from .test_admin import _admin_role_id

ME = identity_path("/v1/me")


def _me(client: ApiClient) -> MeResponse:
    response = client.get(ME)
    assert response.status_code == 200, f"me: {response.status_code} {response.text[:300]}"
    return response.parse(MeResponse)


@pytest.mark.security
def test_an_unauthenticated_caller_never_reaches_any_of_this(api_client: ApiClient) -> None:
    """Proven per operation by `test_gateway.py`; spot-checked here so this
    module carries its own reason for using a session at all."""
    response = api_client.get(ME)
    assert response.status_code == 401, f"{response.status_code} {response.text[:300]}"


@pytest.mark.requires_seed("admin_operator")
@pytest.mark.reliability
def test_the_admin_operator_sees_their_admin_role_under_its_fixed_id(
    admin_operator_session: PersonaSession,
) -> None:
    """The one seeded holder of the `person_roles` row gets it named back.

    Both halves of the item matter: the SPA gates on `role_id` (the stable
    constant), while `name` is what an operator reads in any debugging output —
    a row with the right id and the wrong name means the join broke.
    """
    me = _me(admin_operator_session.client)

    assert str(me.person_id) == admin_operator_session.person.uuid, (
        f"/v1/me answered about {me.person_id}, but the session belongs to "
        f"{admin_operator_session.person.uuid} — the caller must come from the JWT"
    )
    admin_role_id = _admin_role_id(admin_operator_session.client)
    roles = {(str(role.role_id), role.name) for role in me.roles}
    assert (admin_role_id, "admin") in roles, f"admin grant missing from {roles}"


@pytest.mark.reliability
def test_a_lead_without_the_grant_gets_an_empty_list_not_a_refusal(
    lead_session: PersonaSession,
) -> None:
    """200 with `roles: []` IS the "not an admin" answer.

    If this ever becomes a 403, the SPA's boot call starts failing for every
    ordinary user; if it ever lists a role the seed never granted, the admin
    console opens for the whole org.
    """
    me = _me(lead_session.client)

    assert str(me.person_id) == lead_session.person.uuid
    assert me.roles == [], f"seed grants no roles to {lead_session.name}: {me.roles}"


@pytest.mark.requires_seed("ceo")
@pytest.mark.security
def test_the_realm_admin_role_does_not_leak_into_the_answer(
    realm_admin_session: PersonaSession,
) -> None:
    """The CEO holds `insight-admin` in the REALM and no `person_roles` row.

    `require_admin` refuses that persona on every gated route
    (`test_admin.py`), so `/v1/me` must answer them an empty list — one field
    copied from the login token here and the frontend would draw an admin
    console the backend then 403s, which is precisely the confusion this
    endpoint exists to prevent.
    """
    assert realm_admin_session.has_realm_role(ADMIN_ROLE)

    me = _me(realm_admin_session.client)

    assert me.roles == [], (
        f"{realm_admin_session.name} holds {ADMIN_ROLE} in the realm only, yet "
        f"/v1/me lists {me.roles}"
    )
