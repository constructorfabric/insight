"""Identity shapes that no OpenAPI document describes — hand-written.

The service registers its two `/internal/persons/*` S2S resolvers as raw routes,
deliberately kept out of the generated document (the .NET contract excluded them
the same way). They therefore cannot be generated into `identity.py`, and a model
for them has to be written from the Rust DTO by hand.

`extra` stays at its default here, unlike the generated models: nothing
regenerates this file when the DTO gains a field, so forbidding the unknown would
turn a benign addition into a failing suite.
"""

from __future__ import annotations

from uuid import UUID

from pydantic import BaseModel


class IdentityValue(BaseModel):
    """`GET /internal/persons/by-external-id` — the login-bootstrap lookup.

    NOT a `ProfileResponse`, though both are "a person looked up". This route
    answers the identity VALUE that matched — the alias row, pointing at what it
    resolved to — because at login the caller has an identifier and needs to
    learn which person it belongs to, not to read that person's attributes.
    Hence `insight_source_id` rather than `person_id`, and no tenant at all: the
    tenant is exactly what is still unknown at that point.
    """

    value_type: str
    value: str
    insight_source_type: str
    insight_source_id: UUID
