"""Shared contract vocabulary for the identity suite."""

from __future__ import annotations

from typing import Any

import httpx

# Error envelope `type`: the toolkit emits `gts://gts.cf.core.errors.err.v1~…`.
# The body is RFC-9457-shaped ({type,title,status,detail}); tests assert the
# SHAPE + status, and the scheme only as a prefix.
ERROR_TYPE_PREFIXES = ("gts://",)


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


def list_response(body: Any) -> tuple[list[dict[str, Any]], str | None]:
    """Assert the EXACT list wire envelope and return (items, next_cursor).

    Every list endpoint (roles, person-roles, visibility, persons-seed)
    answers `{"items": [...], "next_cursor": null|str}` — verified against the
    live service. This is the consumer contract: a bare JSON array or a
    missing `next_cursor` is a wire break, not a tolerable variation, so no
    normalization happens here.
    """
    assert isinstance(body, dict), f"list envelope must be an object, got: {body!r}"
    assert set(body) >= {"items", "next_cursor"}, f"envelope keys: {sorted(body)}"
    assert isinstance(body["items"], list), body
    cursor = body["next_cursor"]
    assert cursor is None or isinstance(cursor, str), body
    return body["items"], cursor


def items_of(body: Any) -> list[dict[str, Any]]:
    """Items of a strict list envelope (see `list_response`)."""
    return list_response(body)[0]
