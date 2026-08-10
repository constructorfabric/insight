"""Identity shapes the published contract deliberately does not describe.

`identity.py` is generated from `docs/components/backend/identity-resolution/
openapi.json`. The `/internal/*` routes are mounted outside the OpenAPI
registry on purpose — they are service-to-service only and are not part of the
public surface — so nothing in that document describes them and no model for
them can be generated.

Hand-written models therefore describe **observed behaviour**, not a contract:
`extra` stays at its default, because a benign upstream addition should not
fail a suite that never had a contract to check against in the first place.
"""

from __future__ import annotations

from uuid import UUID

from pydantic import BaseModel


class IdentityValue(BaseModel):
    """`GET /internal/persons/by-email/{email}` — the login-bootstrap lookup.

    NOT a `ProfileResponse`, though both are "a person looked up by email".
    This route answers the identity VALUE that matched — the alias row,
    pointing at what it resolved to — because at login the caller has an email
    and needs to learn which person it belongs to, not to read that person's
    attributes. Hence `insight_source_id` rather than `person_id`, and no
    tenant at all: the tenant is exactly what is still unknown at that point.
    """

    value_type: str
    value: str
    insight_source_type: str
    insight_source_id: UUID
