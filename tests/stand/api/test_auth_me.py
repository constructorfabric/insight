"""`GET /auth/me` — the session names the person who actually logged in.

    GET /auth/me   200, naming the caller's email, person id and tenant

A session cookie proves the IdP accepted a credential; it does not by itself
prove identity resolution put the RIGHT person, the right person id or the
right tenant on the session. A stand where every login silently resolved to
the same account, or to the wrong tenant, would pass every other test in this
suite by coincidence — this is the one place that checks the account itself
rather than assuming it from a successful login.

Marked `stand_smoke`: this is the post-deploy gate's "the session belongs to
the person who logged in" check, over three authority levels rather than one
persona, so a stand that resolves logins correctly for a lead but not for an
admin or a member still fails here. The 401 half — no session at all — is
covered in `test_gateway.py`'s sweep.
"""

from __future__ import annotations

import pytest
from insight_stand import Manifest, PersonaSession
from pydantic import BaseModel

# Whole module asserts who a session IS — access/identity scope.
pytestmark = pytest.mark.security

ME_PATH = "/auth/me"


class AuthMeResponse(BaseModel):
    """`GET /auth/me`'s success body, hand-written from observed behaviour.

    The authenticator's own OpenAPI document declares every `/auth/*` success
    body as a bare `type: object` with no properties (see
    `schemas/authenticator.py`'s module docstring), so there is no generated
    contract to validate against here — this is what the route has actually
    been seen to answer. Kept local to this module rather than added to
    `schemas/`: it has exactly one consumer, and a hand-written model beside
    its only test is easier to keep honest than one filed away from the
    assertion that checks it.
    """

    email: str
    user: str
    tenant_id: str


@pytest.fixture(params=["realm_admin_session", "lead_session", "member_session"])
def any_role_session(request: pytest.FixtureRequest) -> PersonaSession:
    """One persona per authority level the manifest actually seeded.

    Parametrized over FIXTURE NAMES rather than sessions, so each case still
    goes through the exact fixture the rest of the suite uses for that role —
    `realm_admin_session`, `lead_session`, `member_session` in
    `tests/stand/conftest.py` — and a roster reshuffle that moves who holds a
    role is picked up here for free, with no role-resolution logic of this
    module's own to keep in sync with theirs.
    """
    return request.getfixturevalue(request.param)


@pytest.mark.stand_smoke
def test_auth_me_names_the_authenticated_persona(
    any_role_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """The session's own account, not just *an* account, is what comes back.

    Asserted against the SEEDED FACTS the login started from — the persona's
    own email and the manifest's UUID for them — rather than only checking the
    response is well-formed. The person id is asserted alongside the email
    because it is the key every person-scoped route takes since the identity
    cutover (#2098).
    """
    response = any_role_session.client.get(ME_PATH)
    assert response.status_code == 200, (
        f"GET {ME_PATH} answered {response.status_code} for {any_role_session.email} while "
        f"carrying a session cookie: {response.text[:300]}"
    )

    body = response.parse(AuthMeResponse)
    assert body.email.casefold() == any_role_session.email.casefold(), (
        f"logged in as {any_role_session.email!r} and {ME_PATH} reports {body.email!r} — the "
        f"session was minted for a different person than the one who authenticated."
    )
    assert body.user == any_role_session.person.uuid, (
        f"{ME_PATH} resolved {any_role_session.email} to person id {body.user!r}, but the "
        f"manifest at {stand_manifest.source_path} says {any_role_session.person.uuid!r}."
    )
    assert body.tenant_id == stand_manifest.tenant, (
        f"{ME_PATH} put tenant {body.tenant_id!r} on {any_role_session.email}'s session, but "
        f"the stand was seeded for {stand_manifest.tenant!r}."
    )
