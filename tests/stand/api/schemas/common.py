"""Shapes both services share: the error envelope and the listing wrapper.

Read from the bodies the stand actually returns, and from the toolkit types the
services build them with — not from a committed OpenAPI document.
"""

from __future__ import annotations

from collections.abc import Sequence
from datetime import datetime

from insight_stand import JsonValue
from pydantic import BaseModel, ConfigDict, Field

#: A timestamp the analytics contract declares as `format: date-time` — RFC 3339,
#: which REQUIRES an offset — but which the service serialises without one:
#: `"2026-07-31T09:12:55"`. Generating faithfully from the spec produces
#: `AwareDatetime` and every response with a timestamp fails to validate.
#:
#: `tests/generate_schemas.py` substitutes this alias for `AwareDatetime` in the
#: generated module — the ONE hand-written thing in a generated file. A pinned
#: DEVIATION, not a preference: any client that generated its parser from the
#: published contract breaks on these too.
#:
#: Delete the substitution and this alias together once the service emits
#: offsets (or the spec stops claiming date-time).
UnzonedDatetime = datetime

#: What the canonical error layer answers with. Asserted alongside the status
#: code, because a proxy that turned a Problem into an HTML page would keep the
#: status and lose everything a client can act on.
PROBLEM_CONTENT_TYPE = "application/problem+json"


class ProblemDocument(BaseModel):
    """RFC 7807 problem document, as the services' canonical error layer emits it.

    `detail` is required rather than optional. RFC 7807 allows omitting it, but
    every error this stand produces carries one, and it is the field that makes a
    rejection actionable — the suite's stated position on the 401 is that "a 401
    a client cannot act on is only half a rejection". An endpoint that stopped
    sending one should fail here rather than pass quietly.

    `instance` and `trace_id` are genuinely optional: the gateway's own 401 omits
    both, while a rejection raised inside a service carries them.
    """

    model_config = ConfigDict(extra="forbid")

    #: A `gts://…` URN identifying the error class.
    type: str
    title: str
    status: int
    detail: str
    #: The request path, on errors raised inside a service.
    instance: str | None = None
    trace_id: str | None = None
    #: Error-class-specific fields (`reason`, `resource_name`, …).
    context: dict[str, JsonValue] = Field(default_factory=dict)


class ListResponse[T](BaseModel):
    """`{"items": [...], "next_cursor": null}` — every listing in both services.

    Generic because it is genuinely one shape: identity says so in its own source
    ("wire parity with the .NET ListResponse: the cursor is declared but
    pagination is not implemented — always null"), and analytics matches.
    """

    items: list[T]
    next_cursor: str | None = None


#: Rejections raised by Axum's own extractors — a path parameter that will not
#: deserialize, a body refused on its media type, a body that is valid JSON but
#: not the request type. They are produced BELOW the canonical error layer, so
#: they arrive as `text/plain` with no problem document at all. Tracked upstream
#: as #1670; asserted as they behave rather than as the spec declares them, and
#: this constant is what a reader greps for when the extractor is made canonical.
EXTRACTOR_REJECTION_CONTENT_TYPE = "text/plain; charset=utf-8"


__all__: Sequence[str] = (
    "EXTRACTOR_REJECTION_CONTENT_TYPE",
    "PROBLEM_CONTENT_TYPE",
    "ListResponse",
    "ProblemDocument",
    "UnzonedDatetime",
)
