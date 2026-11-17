"""Hand-written from the authenticator's Rust (`api/handlers.rs` json! literals, `jwt.rs`,
`service_token.rs`): its OpenAPI document declares these success bodies as bare objects."""

from __future__ import annotations

from typing import Literal
from uuid import UUID

from pydantic import BaseModel, ConfigDict


class AuthMe(BaseModel):
    model_config = ConfigDict(extra="forbid")

    user: UUID
    email: str
    tenant_id: UUID
    roles: list[str]
    expires_at: int
    refresh_at: int
    csrf_token: str
    experiments_enabled: bool
    impersonator_email: str | None = None


class CsrfToken(BaseModel):
    model_config = ConfigDict(extra="forbid")

    csrf_token: str


class Refreshed(BaseModel):
    model_config = ConfigDict(extra="forbid")

    expires_at: int
    refresh_at: int


class LoggedOut(BaseModel):
    model_config = ConfigDict(extra="forbid")

    rp_logout_url: str | None


class SessionSummary(BaseModel):
    model_config = ConfigDict(extra="forbid")

    session_id: UUID
    created_at: int
    expires_at: int
    user_agent: str
    ip: str
    current: bool


class SessionList(BaseModel):
    model_config = ConfigDict(extra="forbid")

    sessions: list[SessionSummary]


class Revoked(BaseModel):
    model_config = ConfigDict(extra="forbid")

    revoked: int


class OpenIdConfiguration(BaseModel):
    model_config = ConfigDict(extra="forbid")

    issuer: str
    jwks_uri: str
    id_token_signing_alg_values_supported: list[str]
    response_types_supported: list[str]
    subject_types_supported: list[str]


class Jwk(BaseModel):
    model_config = ConfigDict(extra="forbid")

    kty: Literal["EC"]
    crv: Literal["P-256"]
    x: str
    y: str
    use: Literal["sig"]
    alg: Literal["ES256"]
    kid: str


class Jwks(BaseModel):
    model_config = ConfigDict(extra="forbid")

    keys: list[Jwk]


class ServiceToken(BaseModel):
    model_config = ConfigDict(extra="forbid")

    access_token: str
    token_type: Literal["Bearer"]
    expires_in: int
