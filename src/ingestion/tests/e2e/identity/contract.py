"""Shared contract vocabulary for the identity suite.

KNOWN DIVERGENCES between the .NET identity service and its Rust
identity-resolution replacement (all reviewed and accepted on epic #1602).
Tests assert against these sets instead of a single implementation's literal
so the SAME suite is green on both sides of the cutover; everything outside
this list is asserted exactly.
"""

from __future__ import annotations

from typing import Any

import httpx

# The .NET service returns 422 for data-invariant guard refusals
# (`ambiguous_profile`, `role_in_use`, `last_admin_protected`); the gears
# canonical-error model has no 422, so the Rust port maps the same guards to
# 409. One accepted divergence family — tests assert membership in this set.
UNPROCESSABLE_OR_CONFLICT = {422, 409}
AMBIGUOUS_STATUSES = UNPROCESSABLE_OR_CONFLICT

# Error envelope `type`: .NET emits `urn:insight:error:<code>`, the Rust
# toolkit emits `gts://gts.cf.core.errors.err.v1~…`. Both are RFC-9457-shaped
# ({type,title,status,detail}); tests assert the SHAPE + status, never the
# scheme.
ERROR_TYPE_PREFIXES = ("urn:insight:error:", "gts://")


def problem(response: httpx.Response) -> dict[str, Any]:
    """Parse an RFC-9457 problem body and assert its common shape:
    type (scheme-agnostic), matching status, a non-empty title, and detail."""
    body = response.json()
    assert isinstance(body, dict), f"problem body is not an object: {body!r}"
    assert body.get("status") == response.status_code, body
    assert str(body.get("type", "")).startswith(ERROR_TYPE_PREFIXES), body
    assert isinstance(body.get("title"), str) and body["title"].strip(), body
    assert "detail" in body, body
    return body


def items_of(body: Any) -> list[dict[str, Any]]:
    """Normalize a list response: `{items: [...]}` or a bare JSON array."""
    if isinstance(body, dict) and "items" in body:
        return list(body["items"] or [])
    assert isinstance(body, list), f"expected a list response, got: {body!r}"
    return body
