"""Previews response shapes — GENERATED, do not edit.

Regenerate with:

    uv run --project tests --frozen python tests/generate_schemas.py

Source: `docs/components/backend/previews/openapi.json`, generated offline by
`cargo run -p previews --bin previews -- openapi` and drift-gated in CI beside
the analytics and identity documents. These models describe the structs that
serialize the wire, so a validation failure is a contract disagreement rather
than a stale transcription.

BODIES ONLY — no status code comes from this document. Its per-operation lists
are stamped uniformly by `.standard_errors` and describe nothing (#1669), the
same limitation the other generated documents carry.
"""

from __future__ import annotations
from pydantic import AwareDatetime, BaseModel, ConfigDict, Field
from enum import StrEnum
from typing import Any


class CreateExperimentRequest(BaseModel):
    """
    Body of `POST /v1/experiments`. The image repository is fixed server-side;
    only the tag varies.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    name: str = Field(..., description='The `/exp/<name>` slug: a DNS-1123 label, at most 55 characters.')
    tag: str = Field(..., description='FE image tag: a `preview-…` tag or a CI build tag.')
    ttlDays: int | None = Field(None, description='Days until the TTL sweep removes the experiment; server default and\nmaximum apply.', ge=0)


class ExperimentStatus(StrEnum):
    ready = 'ready'
    pending = 'pending'
    expired = 'expired'


class ImageListResponse(BaseModel):
    """
    Body of `GET /v1/images`.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    configured: bool = Field(..., description='False when tag listing is disabled server-side; `tags` is then empty.')
    tags: list[str] = Field(..., description="The repository's `preview-…` tags, deduped and sorted.")


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


class ExperimentResponse(BaseModel):
    """
    One experiment as served by the API.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    createdAt: AwareDatetime | None = None
    creator: str = Field(..., description='The creating gateway-JWT subject.')
    expiresAt: AwareDatetime | None = None
    name: str
    status: ExperimentStatus
    tag: str
    url: str = Field(..., description='Where the experiment serves: `https://<host>/exp/<name>/`.')


class ExperimentListResponse(BaseModel):
    """
    List wrapper for `GET /v1/experiments`.
    """
    model_config = ConfigDict(
        extra='forbid',
    )
    cap: int = Field(..., ge=0)
    experiments: list[ExperimentResponse]
    liveCount: int = Field(..., description='Experiments counting against the cap; expired ones do not.', ge=0)
