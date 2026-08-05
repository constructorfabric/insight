"""`/internal/*` — the routes only a SERVICE may call.

Two separate contracts live here, by design (never a shared `value_type`
dispatch), so the login-bootstrap and the admin `__override` view-as feature
can never be confused for one another:

- `GET /internal/persons/by-external-id?source_type=...&external_id=...` —
  the login-bootstrap resolve, scoped to the IdP's `source_type` + its
  source-native external id (e.g. the Entra `oid` claim). This is what the
  authenticator actually calls during login.
- `GET /internal/persons/by-email-override?email=...` — the authenticator's
  admin `__override` (view-as) resolve; never used by login. This is the URL
  the OLD, now-removed `GET /internal/persons/by-email/{email}` login-bootstrap
  lookup would map to if it still existed — it doesn't: this route is
  override-only by contract.

Both are the one route in this suite reached by something other than a
logged-in human, and the only place a service principal appears.

The credential is obtained, not minted: an RFC 7523 assertion signed with the
stand's `testclient` key is exchanged at the authenticator's own token endpoint
for a gateway JWT whose `sub_type` is `service`. See
`tests/lib/insight_stand/service_token.py` for why that distinction is the whole
point of testing this here rather than in the in-process rig.

Both halves matter and neither means anything alone. A 200 for the service
principal is equally consistent with the route being open to anybody
authenticated; a 403 for a person is equally consistent with the route being
broken. Together they say the gate is on the KIND of principal.

They also use two different ADDRESSES, and that is the product, not a
workaround. The gateway is a browser BFF: it delegates authz to the
authenticator, which looks for a session cookie and answers `401 no_session` to
a bearer-carrying request, so a service principal has no edge address at all.
The service therefore calls identity-resolution directly, exactly as the
authenticator does during login, while the human's refusal is asserted at
`/api/identity/...` where a human's request actually arrives. See
`service_token.default_identity_url`.

Skipped, with a reason, on a stand whose token listener this runner cannot
reach — a k8s stand keeps it in-cluster with no ingress. That is what
`requires_service_principal` reads.
"""

from __future__ import annotations

import pytest
from insight_stand import ApiClient, Manifest, PersonaSession, identity_path

from ..schemas import IdentityValue

# The dev lead's fixed external id under fakeidp — mirrors
# `deploy/seed/profiles.py::_FAKEIDP_DEV_LEAD_EXTERNAL_ID`. fakeidp's
# users.yaml pins its first user's `sub` to this value regardless of which
# email the dev lead persona was seeded under, so it cannot be derived from
# the manifest fixture and has to be duplicated here, same as the seed does.
_FAKEIDP_DEV_LEAD_EXTERNAL_ID = "fakeidp|dev"


def _dev_lead_login_id(stand_manifest: Manifest) -> tuple[str, str]:
    """`(source_type, external_id)` the dev lead's login-bootstrap row was
    seeded under, for whichever IdP this stand runs.

    Mirrors `deploy/seed/profiles.py::get_login_id_pairs` /
    `get_idp_source_type`: on `keycloak` every persona's external id is their
    own roster uuid; on `fakeidp` only the dev lead can log in at all, under
    the fixed id above. `capabilities.idp` (`"keycloak"` | `"fakeidp"`) is
    usable directly as `source_type` because compose always seeds
    `AUTHENTICATOR_IDP_SOURCE_TYPE` from the same `AUTH_MODE` (see
    `dev-compose.sh`, `docker-compose.yml`).
    """
    source_type = stand_manifest.capabilities.idp
    person = stand_manifest.fixture("dev_lead")
    external_id = person.uuid if source_type == "keycloak" else _FAKEIDP_DEV_LEAD_EXTERNAL_ID
    return source_type, external_id


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("dev_lead")
def test_by_email_override_serves_a_service_principal(
    service_client: ApiClient, stand_manifest: Manifest
) -> None:
    """The S2S route answers a caller the authenticator actually issued a token to.

    A pass means the whole issuance path works, not merely that identity
    compares a claim.
    """
    person = stand_manifest.fixture("dev_lead")
    response = service_client.get(
        "/internal/persons/by-email-override", params={"email": person.email}
    )
    assert response.status_code == 200, (
        f"the service principal was refused {person.email}: "
        f"{response.status_code} {response.text[:300]}"
    )

    # The alias row, not the person's profile — see `IdentityValue`. Asserting
    # the whole shape rather than just the id: the point of this lookup is
    # that the email it was ASKED about is the one it resolved, and an answer
    # about a different person would satisfy an id-only check whenever the
    # seed happens to have one person.
    resolved = response.parse(IdentityValue)
    assert (resolved.value_type, resolved.value) == ("email", person.email)
    assert resolved.insight_source_type == "person"
    assert str(resolved.insight_source_id) == person.uuid


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("dev_lead")
def test_by_email_override_refuses_a_person(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """A logged-in human is refused the same route.

    The half that makes the test above mean something: without it, a 200 for
    the service principal would be equally consistent with the route being open
    to anybody authenticated. Same url, same tenant, same seeded person —
    differing only in what kind of principal is asking.
    """
    person = stand_manifest.fixture("dev_lead")
    response = lead_session.client.get(
        identity_path("/internal/persons/by-email-override"), params={"email": person.email}
    )
    assert response.status_code == 403, (
        f"a person reached the service-only route (status {response.status_code}) — "
        f"/internal/* is restricted to sub_type=service: {response.text[:300]}"
    )


@pytest.mark.requires_service_principal
def test_by_email_override_of_an_unknown_email_is_404(service_client: ApiClient) -> None:
    """An address nobody holds, asked by the one caller entitled to ask.

    A person who has never been seen must be reported as absent rather than as
    a refusal — a 403 here would make an unknown email indistinguishable from a
    misconfigured service credential.
    """
    response = service_client.get(
        "/internal/persons/by-email-override", params={"email": "nobody@example.com"}
    )
    assert response.status_code == 404, (
        f"an unknown email answered {response.status_code} to a service principal: "
        f"{response.text[:300]}"
    )


@pytest.mark.requires_service_principal
def test_by_email_override_missing_email_is_400(service_client: ApiClient) -> None:
    """The query param is required — an absent `email` is a bad request, not a
    404, so a caller that forgot the param does not read as "unknown email"."""
    response = service_client.get("/internal/persons/by-email-override")
    assert response.status_code == 400, (
        f"a missing email answered {response.status_code}, not 400: {response.text[:300]}"
    )


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("dev_lead")
def test_by_external_id_serves_a_service_principal(
    service_client: ApiClient, stand_manifest: Manifest
) -> None:
    """The login-bootstrap resolve: the dev lead's seeded `value_type='id'`
    row, scoped to this stand's active IdP `source_type`."""
    person = stand_manifest.fixture("dev_lead")
    source_type, external_id = _dev_lead_login_id(stand_manifest)
    response = service_client.get(
        "/internal/persons/by-external-id",
        params={"source_type": source_type, "external_id": external_id},
    )
    assert response.status_code == 200, (
        f"the service principal was refused external_id={external_id!r} "
        f"under source_type={source_type!r}: {response.status_code} {response.text[:300]}"
    )

    resolved = response.parse(IdentityValue)
    assert (resolved.value_type, resolved.value) == ("id", external_id)
    assert resolved.insight_source_type == "person"
    assert str(resolved.insight_source_id) == person.uuid


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("dev_lead")
def test_by_external_id_refuses_a_person(
    lead_session: PersonaSession, stand_manifest: Manifest
) -> None:
    """Same service-only gate as `by-email-override`, asserted on this route
    too — the two are separate routes with independent guards, and one
    passing the gate says nothing about the other."""
    source_type, external_id = _dev_lead_login_id(stand_manifest)
    response = lead_session.client.get(
        identity_path("/internal/persons/by-external-id"),
        params={"source_type": source_type, "external_id": external_id},
    )
    assert response.status_code == 403, (
        f"a person reached the service-only route (status {response.status_code}) — "
        f"/internal/* is restricted to sub_type=service: {response.text[:300]}"
    )


@pytest.mark.requires_service_principal
def test_by_external_id_of_an_unknown_id_is_404(stand_manifest: Manifest, service_client: ApiClient) -> None:
    response = service_client.get(
        "/internal/persons/by-external-id",
        params={"source_type": stand_manifest.capabilities.idp, "external_id": "nobody-external-id"},
    )
    assert response.status_code == 404, (
        f"an unknown external id answered {response.status_code} to a service principal: "
        f"{response.text[:300]}"
    )


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("dev_lead")
def test_by_external_id_missing_source_type_is_400(
    service_client: ApiClient, stand_manifest: Manifest
) -> None:
    _, external_id = _dev_lead_login_id(stand_manifest)
    response = service_client.get(
        "/internal/persons/by-external-id", params={"external_id": external_id}
    )
    assert response.status_code == 400, (
        f"a missing source_type answered {response.status_code}, not 400: {response.text[:300]}"
    )


@pytest.mark.requires_service_principal
def test_by_external_id_missing_external_id_is_400(
    service_client: ApiClient, stand_manifest: Manifest
) -> None:
    response = service_client.get(
        "/internal/persons/by-external-id", params={"source_type": stand_manifest.capabilities.idp}
    )
    assert response.status_code == 400, (
        f"a missing external_id answered {response.status_code}, not 400: {response.text[:300]}"
    )


@pytest.mark.requires_service_principal
@pytest.mark.requires_seed("dev_lead")
def test_by_external_id_never_resolves_by_email(
    service_client: ApiClient, stand_manifest: Manifest
) -> None:
    """A login-mode request carrying an email-shaped value in `external_id`
    must NOT resolve via any email fallback — `by-external-id` and
    `by-email-override` are separate routes, not a shared dispatch, so this
    must 404 like any other unknown external id."""
    person = stand_manifest.fixture("dev_lead")
    response = service_client.get(
        "/internal/persons/by-external-id",
        params={"source_type": stand_manifest.capabilities.idp, "external_id": person.email},
    )
    assert response.status_code == 404, (
        f"an email-shaped external_id resolved (status {response.status_code}) instead of "
        f"404ing like any other unknown id: {response.text[:300]}"
    )
