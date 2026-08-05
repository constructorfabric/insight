"""Authenticator response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/authenticator/openapi.json`, generated offline
by `cargo run -p authenticator -- openapi` and drift-gated in CI beside the
analytics document — same provenance, same guarantee that these models describe
the structs that serialize the wire.

SMALL ON PURPOSE, and it is the document that is small: every success body on
`/auth/*` is declared as a bare `type: object` with no properties, because those
handlers answer untyped JSON. So what the contract describes today is the error
envelope, and that is what this module holds. A handler that gains a typed
response appears here on the next regeneration — which is the reason this file
exists rather than the service being listed as having nothing to generate.

Two consequences worth knowing while it stays this small:

* The envelope here is the CONTRACT's, generated and drift-gated; the one in
  `common.ProblemDocument` is hand-written from observed bodies and is what the
  suite asserts with today. They agree field for field, with one difference: the
  contract REQUIRES `context`, where the hand-written model defaults it to `{}`.
* No status code comes from this document either — the same `.standard_errors`
  stamping applies (#1669).
"""

from __future__ import annotations
from typing import Any
from pydantic import BaseModel, ConfigDict


class Problem(BaseModel):
    """
    RFC 9457 problem+json. `context` varies by error category.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    context: dict[str, Any]
    detail: str
    instance: str | None = None
    status: int
    title: str
    trace_id: str | None = None
    type: str
