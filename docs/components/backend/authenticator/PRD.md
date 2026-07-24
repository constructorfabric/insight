---
status: proposed
date: 2026-07-06
---

# PRD -- Authenticator Service

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 OIDC Login Flow](#51-oidc-login-flow)
  - [5.2 Session Identity and Credential Split](#52-session-identity-and-credential-split)
  - [5.3 Session Cookie](#53-session-cookie)
  - [5.4 Session Refresh](#54-session-refresh)
  - [5.5 Session Store](#55-session-store)
  - [5.6 Linked Gateway JWT](#56-linked-gateway-jwt)
  - [5.7 Cookie-to-JWT Exchange](#57-cookie-to-jwt-exchange)
  - [5.8 JWKS Publication](#58-jwks-publication)
  - [5.9 Session Management](#59-session-management)
  - [5.10 Logout](#510-logout)
  - [5.11 CSRF Protection](#511-csrf-protection)
  - [5.12 IdP Token Lifecycle](#512-idp-token-lifecycle)
  - [5.13 Service Tokens](#513-service-tokens)
  - [5.14 Bootstrap of a Fresh Install](#514-bootstrap-of-a-fresh-install)
  - [5.15 Internal Endpoint Reachability](#515-internal-endpoint-reachability)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [8.1 Browser Session Lifecycle](#81-browser-session-lifecycle)
  - [8.2 Service-to-Service Authentication](#82-service-to-service-authentication)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The authenticator is the auth core of Insight: a standalone service that implements the BFF / token-handler pattern. It runs the OIDC login flow against the customer's identity provider, holds the user session server-side in Redis, and exposes a small `/auth/*` API the SPA uses to log in, refresh, and log out. IdP tokens never leave the authenticator; the browser holds only an opaque session cookie.

At login the authenticator mints a signed **gateway JWT linked 1:1 to the session** -- the complete, signed description of the request author (`sub` = person_id, `tenant_id`, `roles`, `sid`). On every API request the nginx gateway (see [Gateway DESIGN](../gateway/DESIGN.md)) exchanges the session cookie for that JWT via an `auth_request` subrequest to `GET /internal/authz`; only the JWT travels to downstream services, which verify it against the authenticator's JWKS. The authenticator also issues **service tokens** for no-user workloads, so downstream services keep exactly one verification path.

### 1.2 Background / Problem Statement

Today the SPA runs the OIDC flow itself with `oidc-client-ts`, keeps IdP tokens in browser `sessionStorage`, and sends `Authorization: Bearer <IdP token>` on every request. Any XSS leaks every active token; tokens are visible to extensions and dev tools; revocation requires waiting out the IdP token lifetime; and downstream services trust an unsigned `X-Insight-Tenant-Id` header for tenant context.

The previously specified remedy -- a single Rust API Gateway binary combining a BFF module with a custom reverse-proxy Router module -- was removed by decision (see the nginx + authorization decision document): the proxy half is commodity work nginx does better, while the BFF half is the hard, security-critical, custom part. The authenticator is that BFF half, kept as a standalone service reached by nginx subrequests, with deliberate changes: the JWT is minted at login and linked to the session (not lazily per request), it carries `tenants` and `roles` claims (superseding the identity-only contract), IdP tokens are refreshed in the background (reversing the earlier "no IdP refresh in v1" carve-out), and the session's stable identity is split from its rotating cookie credential.

### 1.3 Goals (Business Outcomes)

- Remove all IdP and access tokens from browser storage; the browser holds one opaque cookie.
- Make sessions revocable per-session and per-user from a single store, with gateway-visible effect bounded by a configurable cache max-age (default 30 s).
- Give every internal service a verifiable, short-lived, signed identity claim per request -- for user traffic and service-to-service traffic alike.
- Propagate IdP-side account deactivation within roughly one IdP access-token lifetime even when the customer IdP has no back-channel logout.
- Keep the SPA simple: no token handling code in the browser.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Session | The server-side login of one user on one device, born at `/auth/callback`, dead at logout/revoke/expiry. Identified by a **stable `session_id`** (UUIDv7) that never changes across cookie rotations. |
| Session token | The opaque cookie value -- a rotating **credential** mapping to the `session_id`. Generated from a CSPRNG; no claims, no meaning outside Redis. Rotation writes a new mapping and lets the old one expire after a grace TTL. |
| Gateway JWT | Short-lived signed JWT minted by the authenticator **at login**, stored server-side, linked 1:1 to the session, reissued ahead of expiry. The only credential downstream services ever see. |
| Exchange | The gateway's `auth_request` subrequest to `GET /internal/authz`: session cookie in, `X-Gateway-Jwt` header out. |
| Service token | A gateway JWT for workloads with no user context (`sub_type = "service"`, `sid = service:<name>`), issued at `POST /internal/token` against an RFC 7523 signed assertion. |
| IdP tokens | Tokens issued by the customer's identity provider. Stored only inside the authenticator's session record; refreshed by a background worker; never sent to the browser or downstream services. |
| Downstream service | Any internal Insight service behind the gateway (Analytics API, Identity Service, etc.). Verifies the gateway JWT itself -- mandatory, fail closed, no production disable knob. |

## 2. Actors

### 2.1 Human Actors

#### Browser User

**ID**: `cpt-insightspec-actor-browser-user`

**Role**: Any authenticated end user accessing Insight through the SPA.
**Needs**: Log in, stay logged in across requests, log out, see active sessions, revoke a session from another device.

#### Tenant Administrator

Defined in the [parent backend PRD](../specs/PRD.md) as `cpt-insightspec-actor-tenant-admin`. In this module the Tenant Administrator additionally needs to revoke any user's sessions (forced logout on role change, offboarding, suspected compromise).

### 2.2 System Actors

#### OIDC Provider

Defined in the [parent backend PRD](../specs/PRD.md) as `cpt-insightspec-actor-oidc-provider`. In this module the customer identity provider runs the authorization code + PKCE flow, issues refresh tokens to the authenticator, and may call back-channel logout.

#### Nginx Gateway

**ID**: `cpt-insightspec-actor-nginx-gateway`

**Role**: The edge reverse proxy (see [Gateway DESIGN](../gateway/DESIGN.md)). Calls `GET /internal/authz` per request (through its exchange cache), forwards `/auth/*` traffic verbatim, injects the returned JWT upstream.

#### Downstream Service

**ID**: `cpt-insightspec-actor-downstream-service`

**Role**: Any internal Insight service that receives the gateway JWT and authorizes the request from its signed claims. Also a client of `POST /internal/token` for background work.

#### Redis

**ID**: `cpt-insightspec-actor-redis`

**Role**: Stores session records, token mappings, linked JWTs, and indexes. The single source of truth for "who is logged in".

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Single deployment per Insight installation, reached only through the nginx gateway for browser traffic; TLS terminates at the ingress in front of the gateway.
- The authenticator and the SPA share one public hostname (required by the `__Host-` cookie prefix); the SPA is routed through the gateway.
- Stateless and horizontally scalable -- all session state is in Redis. Background workers (IdP refresher, janitor) elect one leader via a Redis lock.
- Two listeners: the main port (`/auth/*`, JWKS, `/internal/authz`) is network-scoped to the gateway pods; the token port (`POST /internal/token`) is reachable from application-namespace service pods. See 5.15.
- The service name is deliberately dash-free (`authenticator`) so the `APP__gears__authenticator__config__*` environment override convention works in docker compose.

## 4. Scope

### 4.1 In Scope

- OIDC authorization code + PKCE login flow as a confidential client, with session-fixation guard.
- Opaque session cookie with short hard TTL, hardened attributes, and rotation-with-grace on explicit refresh.
- Stable-session-identity / rotating-credential split of the session model.
- Session record storage in Redis with per-user index, IdP-sid index, and atomic pipelines.
- Gateway JWT minted at login, linked 1:1 to the session, reissued ahead of expiry; claim contract with `sub`, `tenants`, `roles`, `sid`.
- Cookie-to-JWT exchange endpoint `GET /internal/authz` for the gateway's `auth_request`, including the response caching contract.
- JWKS publication at `/.well-known/jwks.json`.
- Session listing and revocation (single, all-but-current, all; admin-initiated).
- Logout: local, RP-initiated, and OIDC back-channel receiver with `jti` replay guard.
- CSRF defense for state-changing `/auth/*` requests.
- Background IdP token refresh per session; definitive refusal kills all linked sessions.
- Service tokens at `POST /internal/token` against an RFC 7523 assertion and a gitops-reviewable service registry.
- First-admin bootstrap on a fresh install (guardrailed, off-switchable).
- Rate limiting layer 2 (login-state cap and per-session/user token buckets); audit events; expired-index janitor.

### 4.2 Out of Scope

- Reverse-proxying, route tables, header rewriting, response streaming -- owned by the [nginx gateway](../gateway/DESIGN.md).
- Authorization decisions inside downstream services (each service still enforces RBAC and visibility from signed claims).
- Permissions storage and management -- a separate permissions service, built later. Until it exists every JWT carries the configured default roles.
- User registration, password management, MFA -- handled by the customer OIDC provider.
- WebSocket lifetime enforcement -- deferred to the downstream WS handler when a WS feature lands; no WebSocket code exists in the stack today.
- Mobile or third-party API clients (v1 serves the bundled SPA and internal services).

## 5. Functional Requirements

### 5.1 OIDC Login Flow

#### Authorization Code with PKCE

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-oidc-login`

The system **MUST** implement OIDC authorization code flow with PKCE as a confidential client. The authenticator **MUST** generate `state`, `nonce`, and PKCE verifier per login attempt and validate them on callback. The browser **MUST NOT** receive or transmit the IdP code, ID token, access token, or refresh token at any point.

The new session token issued at the end of a successful callback **MUST** be generated server-side from a CSPRNG and **MUST NOT** be derived from, or equal to, any value present in the incoming request (cookies, headers, query). Any `__Host-sid` cookie present on the `/auth/callback` request **MUST** be ignored; if its value maps to a live session in Redis, that session **MUST** be revoked before the new session is created. This prevents session fixation where an attacker plants a known token before the victim logs in.

At login the system **MUST** resolve the authenticated person via Identity Service (`sub` to `person_id` plus tenant memberships) and **MUST** fetch access-control claims once, from the permissions service when it exists; until then the configured `authenticator.default_roles` apply.

**Rationale**: The whole point of the redesign -- IdP tokens never leave the server; claims are resolved once, at login, and baked into the session.

**Actors**: `cpt-insightspec-actor-browser-user`, `cpt-insightspec-actor-oidc-provider`

### 5.2 Session Identity and Credential Split

#### Stable Session Identity, Rotating Credential

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-session-model`

The system **MUST** separate the session's identity from its credential:

1. **`session_id`** -- stable, internal, generated as UUIDv7 at login, unchanged until logout/revoke/expiry. Everything server-side keys on it: the session record, the linked JWT, the per-user index, the IdP-sid index, audit, and the JWT's `sid` claim.
2. **Session token** (the cookie value) -- a rotating credential stored only as a mapping from token to `session_id`. Refresh rotation writes a new mapping and lets the old mapping expire after a grace TTL. Nothing else moves.

The system **MUST NOT** use the cookie value as a Redis session key, **MUST NOT** rename session records on rotation, and **MUST NOT** maintain a separate swap/grace key family -- the expiring old mapping is the grace window.

**Rationale**: The earlier spec conflated cookie value, session id, Redis key, and `sid` claim, which forced a rename-based rotation pipeline and a stale-`sid` problem. The split removes both: the JWT never carries a stale `sid`, and rotation is one write.

**Actors**: `cpt-insightspec-actor-browser-user`

### 5.3 Session Cookie

#### Session Cookie Issuance

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-session-cookie`

After a successful OIDC callback, the system **MUST** issue an opaque session cookie with these attributes:

- `__Host-` prefix (forces host-only + Secure + Path=/).
- `HttpOnly`.
- `Secure`.
- `SameSite=Strict`.
- Random value with at least 128 bits of entropy.
- Short hard TTL, configurable (`authenticator.session_ttl_seconds`, default 600 seconds). The cookie `Max-Age` **MUST** match the token mapping TTL in Redis.
- The TTL **MUST NOT** be extended automatically by activity. Only an explicit `POST /auth/refresh` extends it (see 5.4).
- An absolute hard cap (`authenticator.session_absolute_lifetime_seconds`, default 8 h) **MUST** apply across refreshes -- once reached, refresh fails and the user must log in again.

The cookie value **MUST** be opaque -- no claims, no JWT, no user-identifying data.

**Rationale**: Short TTL plus explicit refresh limits the window for stolen-cookie reuse; the absolute cap forces re-authentication on a known schedule.

**Actors**: `cpt-insightspec-actor-browser-user`

### 5.4 Session Refresh

#### Explicit Session Refresh Endpoint

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-session-refresh`

The system **MUST** expose `POST /auth/refresh`. The cookie value rotates on every successful refresh. Behaviour:

1. **Unknown token / no session** -- 401, clear the cookie.
2. **Token maps to a live session** (normal path): generate a fresh session token (CSPRNG, at least 128 bits); compute `new_exp = min(now + session_ttl, absolute_expires_at)`; write the new token mapping; shorten the old mapping's TTL to the rotation grace (`authenticator.refresh_grace_ms`, default 250 ms); update the session record's `expires_at`, its Redis TTL, and the per-user index score -- all in one pipeline. Re-issue the cookie and return `200 {expires_at, refresh_at}`.
3. **Token maps to a session already rotated past** (grace path): the old mapping still resolves to the same `session_id` during the grace TTL; return the current cookie value and `200 {expires_at, refresh_at}` without rotating again.

`refresh_at` is server-supplied: `expires_at - safety_margin + jitter`, with `safety_margin` = `authenticator.session_refresh_safety_margin_seconds` (default 90 s) and full jitter window `authenticator.refresh_jitter_seconds` (default 120 s, uniform +/- 60 s), re-jittered on every refresh. The jitter is deliberately big: it spreads refresh load from NAT'd offices into a uniform trickle and keeps an attacker from aligning to the rotation grace window; the late jitter edge still leaves at least 30 s of session life.

The system **MUST NOT** extend the session on any other endpoint. A stale cookie on `/internal/authz` returns 401 immediately -- the SPA must call `/auth/refresh` first. Session refresh **MUST NOT** touch the linked JWT: the JWT is keyed by the stable `session_id` and has its own reissue cycle (see 5.6).

`GET /auth/me` **MUST** return the same `{expires_at, refresh_at}` fields (freshly jittered) so the SPA can prime its refresh timer at page load.

**SPA contract.** The SPA **MUST** coordinate `/auth/refresh` across browser tabs (single leader via `BroadcastChannel`, `localStorage` fallback) and **MUST** schedule the next refresh from the server-supplied `refresh_at`.

**Rationale**: Cookie rotation makes stolen-credential reuse noisy and short-lived; the grace mapping absorbs benign races; the big jitter both spreads load and defeats grace-window alignment.

**Actors**: `cpt-insightspec-actor-browser-user`

### 5.5 Session Store

#### Redis-Backed Session Storage

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-session-store`

The system **MUST**:

1. **Persist sessions** -- record every active session server-side with all fields needed to validate, refresh, and revoke it (person, tenants, roles snapshot, IdP linkage, IdP refresh token and expiries, timestamps, hard cap, CSRF token). Key family `asm:session:{session_id}`.
2. **Maintain the token-credential mapping** `asm:token:{token}` to `session_id`, TTL-bounded (see 5.2, 5.4).
3. **Store the linked JWT** at `asm:jwt:{session_id}` (see 5.6).
4. **Maintain a per-user session index** for "list my devices" and "log out everywhere" in sub-linear time. Key family `asm:user_sessions:{person_id}`.
5. **Maintain an IdP-sid lookup** resolving `(iss, idp_sid)` from back-channel logout tokens to local sessions. Key family `asm:sid_index:*`.
6. **Maintain the IdP refresh schedule** `asm:idp_refresh_due` so the background refresher can find sessions due for refresh without scanning (see 5.12).
7. **Make create / refresh / revoke atomic** -- session record, token mapping, linked JWT, and all indexes change in one pipeline; a partial failure **MUST NOT** leave them out of sync. Revocation deletes session **and** linked JWT together.
8. **Run a periodic janitor** that trims expired entries from the indexes and emits a drift metric.

The exact Redis schema is specified in [DESIGN section 3.7](./DESIGN.md#37-database-schemas--tables).

**Rationale**: Server-side storage is what makes sessions revocable; the indexes make listing, revocation, back-channel logout, and scheduled IdP refresh fast; atomicity prevents zombie state.

**Actors**: `cpt-insightspec-actor-redis`

### 5.6 Linked Gateway JWT

#### Login-Minted, Session-Linked JWT

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-linked-jwt`

The system **MUST** mint the gateway JWT at `/auth/callback`, in the same pipeline that creates the session, and store it keyed by the stable `session_id`. From then on the JWT is **reissued ahead of expiry**: while its age is under `authenticator.jwt_reissue_after_seconds` (default 240 s of the 300 s TTL) the stored JWT is served as-is; past that age the system rebuilds claims from the session record, signs a fresh JWT, and stores it with `SET ... NX EX` so parallel requests converge on one canonical JWT (stampede-safe).

**Guarantee**: the gateway never receives a JWT with less than `jwt_ttl - jwt_reissue_after` (default 60 s) of validity left -- a request can sit in a queue, retry, or travel across services for a minute and still verify downstream.

The JWT **MUST** carry exactly:

| Claim | Value |
|---|---|
| `sub` | internal **person_id** (for service tokens: a stable per-service UUIDv5, see 5.13) |
| `tenant_id` | the **single** tenant this token is scoped to — the sole tenant authority (one and only one tenant per token, EPIC #1583; supersedes the earlier `tenants` array + `X-Tenant-ID` selector sketch — see [DESIGN section 3.8](./DESIGN.md#38-gateway-jwt-claim-contract)) |
| `roles` | present from day one, default from `authenticator.default_roles` (`["user"]`). Once the separate permissions service exists, its answer -- fetched once at login -- replaces the default. Claim shape is fixed now so extending costs nothing |
| `sid` | the **stable** `session_id` (UUIDv7) -- survives cookie rotations; one id from login to logout for tracing, audit, and the JWT/session linkage |
| `iss`, `aud`, `iat`, `exp`, `jti` | `exp = iat + 60..300 s`; `jti` UUIDv7 |

Claim freshness: access-control claims are a login-time snapshot stored in the session record; reissue rebuilds the JWT with fresh `iat`/`exp`/`jti` and the same claims. Permission changes propagate on re-login, or immediately if the permissions service revokes the user's sessions on change.

Session revoke/logout **MUST** delete the session and the linked JWT in one pipeline. Signing keys are a mounted secret with `current` + `previous` overlap for rotation; the signature algorithm decision (EdDSA vs ES256) is recorded as open in [DESIGN section 5](./DESIGN.md#5-design-decisions).

**Rationale**: An eagerly minted, session-linked JWT gives every request a complete signed author description with zero per-request minting on the hot path, and supersedes the earlier identity-only, lazily-minted contract.

**Actors**: `cpt-insightspec-actor-downstream-service`, `cpt-insightspec-actor-nginx-gateway`

### 5.7 Cookie-to-JWT Exchange

#### `/internal/authz` Exchange Endpoint

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-authz-exchange`

The system **MUST** expose `GET /internal/authz` on the main listener as the gateway's `auth_request` target:

1. Read the `__Host-sid` cookie, resolve `asm:token:{token}` to `session_id`, load the session; on miss return **401**.
2. On hit, read the linked JWT; serve it as-is while fresh, reissue ahead of expiry per 5.6.
3. Respond `200` with the JWT in the `X-Gateway-Jwt` response header (`Bearer <jwt>`).
4. A `200` response **MUST** carry `Cache-Control: max-age = min(authz_cache_max_age, jwt_exp - now - 60 s)` so the gateway-side exchange cache can never serve a JWT past its travel margin. Any non-200 response **MUST** carry `Cache-Control: no-store` -- a cached 401 would lock out a user who just logged in.

The response carries no correlation id -- it is cacheable, so per-request correlation ids are generated at the edge (see [Gateway DESIGN](../gateway/DESIGN.md)).

**Rationale**: This endpoint replaces the deleted Router's in-process session check + JWT injection; the Cache-Control contract keeps the authenticator in control of gateway-side staleness (revocation takes effect at the gateway within `authz_cache_max_age`, default 30 s).

**Actors**: `cpt-insightspec-actor-nginx-gateway`

### 5.8 JWKS Publication

#### JWKS Endpoint

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-jwks`

The system **MUST** publish the public verification key set at `GET /.well-known/jwks.json` (RFC 7517), served through the gateway. Each downstream service is configured with the absolute JWKS URL (Helm value, env `GATEWAY_JWKS_URL`); services fetch at startup, cache, and re-fetch on unknown `kid`. Key rotation keeps `current` + `previous` published for the documented overlap window.

**Rationale**: Downstream services verify signatures with no shared secrets and no service discovery.

**Actors**: `cpt-insightspec-actor-downstream-service`

### 5.9 Session Management

#### List Active Sessions

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-session-list`

The system **MUST** expose an authenticated endpoint returning the calling user's active sessions (created_at, expires_at, user_agent, ip, current=true/false), read from the per-user index with score > now.

**Actors**: `cpt-insightspec-actor-browser-user`

#### Revoke Sessions

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-session-revoke`

The system **MUST** support: (1) revoke the current session (logout); (2) revoke a specific other session; (3) revoke all sessions for a user (self "log out everywhere", admin-initiated, or permissions-service-initiated on grant change). Each operation deletes the session record, the linked JWT, the token mapping(s), and the index entries atomically.

Revocation is instant at the authenticator. At the gateway it takes effect within the exchange-cache max-age (default 30 s); any in-flight JWT dies within its own `exp` (at most 300 s).

The admin surface (revoke by user) **MUST** itself require a valid gateway JWT with an authorized role -- the authenticator verifies its own tokens exactly like any downstream service.

**Rationale**: Instant revocation is the reason sessions are opaque and server-side; the admin path is how the future permissions service forces claim refresh.

**Actors**: `cpt-insightspec-actor-browser-user`, `cpt-insightspec-actor-tenant-admin`, `cpt-insightspec-actor-downstream-service`

### 5.10 Logout

#### Logout (Local, RP-Initiated, Back-Channel)

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-logout`

The system **MUST** provide `POST /auth/logout` that revokes the current session, clears the cookie (`Max-Age=0`), and redirects (or returns a redirect URL) to the OIDC `end_session_endpoint` for RP-initiated logout.

The system **MUST** accept OIDC back-channel logout tokens at a dedicated endpoint, validate the `logout_token` per spec, locate sessions by `(iss, sid)` (via the sid index) or `(iss, sub)` (via the sub index `asm:sub_index:*`, maintained at login — Identity cannot resolve a `sub` without an email, and the logout path must not depend on another service), and revoke them.

The system **MUST** protect the back-channel endpoint against replay: every accepted `logout_token` **MUST** be recorded by `(iss, jti)` with a TTL of at least `iat + max_clock_skew`, and any subsequent delivery of the same `(iss, jti)` **MUST** short-circuit to a successful response without performing another revoke.

The system **MUST** document and accept that a `logout_token` carrying only `sub` (no `sid`) revokes every active session for that user ("log out everywhere") -- OIDC-spec-compliant fallback, called out in the runbook so a misconfigured IdP does not silently widen blast radius.

**Rationale**: Back-channel logout is the fast path for IdP-side termination (the background refresher is the guaranteed path, see 5.12); `jti` replay protection stops a captured logout token from being replayed as a DoS.

**Actors**: `cpt-insightspec-actor-oidc-provider`, `cpt-insightspec-actor-browser-user`

### 5.11 CSRF Protection

#### CSRF Defense

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-csrf`

For state-changing methods (POST, PUT, PATCH, DELETE) on `/auth/*`, the system **MUST** require either:

1. A double-submit CSRF token sent in `X-CSRF-Token` matching a value bound to the session, or
2. A verified `Origin` header matching the configured SPA origin.

`SameSite=Strict` is the primary defense; this requirement is the second line.

**Rationale**: Defense in depth is cheap and protects against same-site-but-different-path vectors.

**Actors**: `cpt-insightspec-actor-browser-user`

### 5.12 IdP Token Lifecycle

#### Background IdP Token Refresh

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-idp-refresh`

The session must not outlive the IdP's willingness to vouch for the user. Therefore, deliberately reversing the earlier "no IdP token refresh in v1" carve-out:

1. At login the system **MUST** store the IdP **refresh token** (and access-token expiry) in the session record, alongside the `id_token`.
2. A **background worker** (one leader elected via Redis lock) **MUST** refresh each session's IdP tokens `authenticator.idp.refresh_safety_margin_seconds` before expiry and store the rotated refresh token back. A per-session lock **MUST** guard each refresh: most IdPs rotate refresh tokens one-time-use, and two pods racing the same rotation would burn the grant and falsely kill the session.
3. The worker **MUST** find due sessions via the `asm:idp_refresh_due` schedule (no scanning), with due-times jittered at write so sessions do not herd after a deploy or Redis restore. In-flight refreshes are capped by `authenticator.idp.refresh_concurrency` (default 128) -- politeness toward the customer IdP; IdP `429` is handled as transient honoring `Retry-After`.
4. **Definitive refusal** (`invalid_grant`: revoked, expired, user disabled) **MUST** kill every session linked to that grant through the same revoke pipeline as logout. **Transient failures** (IdP unreachable, timeout, 5xx) **MUST NOT** kill sessions: retry with backoff until success or a definitive verdict. Fail open on transport, fail closed on verdict.
5. When the IdP issues no refresh token, `authenticator.idp.no_refresh_token_policy` applies: `strict` (default) caps the session at the IdP access-token lifetime; `login_only` lets sessions live to the absolute cap, killed only by back-channel logout or manual revoke.
6. The system **MUST** emit metrics for refresh outcomes, a consecutive-transient-failure gauge, and an `invalid_grant` counter -- alert before the mass logout, not after.

**Rationale**: IdP-side deactivation propagates within roughly one IdP access-token lifetime even if the customer IdP has no back-channel logout; a five-minute IdP blip must not log out the entire installation.

**Actors**: `cpt-insightspec-actor-oidc-provider`, `cpt-insightspec-actor-redis`

### 5.13 Service Tokens

#### Service Token Issuance

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-service-tokens`

For workloads with no user context (background jobs, seeds, the future permissions service), the system **MUST** expose `POST /internal/token` on the dedicated token listener:

1. The caller proves its identity with a short-lived signed assertion (`private_key_jwt`, RFC 7523): `iss = sub = <service>`, `aud = authenticator`, `jti`, `exp` at most 60 s.
2. The authenticator validates the signature against a **service registry** -- a gitops-reviewable config mapping service name to public key(s), allowed extra roles, and whether the service may request a tenant-scoped token. Onboarding a service is a PR adding its public key; rotation ships key n+1 alongside n.
3. The assertion `jti` **MUST** be replay-guarded (same `SET NX` pattern as the back-channel logout guard). Every issuance is audited.
4. Out comes a **normal gateway JWT**: `sub` = a stable per-service UUIDv5, `sub_type = "service"`, `sid = service:<name>`, `roles: ["service", ...]` per the registry, TTL 300 s, signed with the same key, published in the same JWKS -- downstream services keep exactly one verification path for user and service traffic. Service tokens are always tenant-scoped: the request names exactly one tenant, carried as the signed `tenant_id`.
5. Services cache the token and re-request before expiry -- the same reissue-ahead pattern as everything else.

User-context fan-out (service A calls service B while serving a user request) does **not** use this endpoint: internal services propagate the incoming `Authorization` header on outbound calls made on behalf of the request -- that is what the 60 s reissue-ahead travel margin buys.

JWT-free internal endpoints behind NetworkPolicy alone are **rejected**: every service verifies the JWT, no exceptions; network position is never trust.

**Rationale**: No secret in transit, reviewable onboarding, one verification path.

**Actors**: `cpt-insightspec-actor-downstream-service`

### 5.14 Bootstrap of a Fresh Install

#### First Login on an Empty Install

- [ ] `p1` - **ID**: `cpt-insightspec-fr-auth-bootstrap`

Login resolves the person via Identity Service; unknown person means 403. A fresh install has an empty persons table -- nobody could ever log in. The system **MUST** support two composable ways in:

1. **Empty table, first admin** (`authenticator.bootstrap_first_admin`, default `true`): if the persons table is empty at login, the IdP-authenticated person is auto-created with the universe-admin role. Guardrails: it admits only someone who authenticated at the customer's IdP; it is active **only while the table is empty** -- the window closes permanently on the first created person; every use emits a loud audit event and log line. Security-sensitive installs can turn it off.
2. **INSTALLER** -- a separate setup component that populates identity before first login (from AD, Okta, CSV, etc.) with explicit admin designation. This is the production path; the persons table must be populated anyway for the product to function.

**Rationale**: Way 1 makes dev, demos, and honest mistakes self-healing; way 2 is the documented production install step, after which way 1's window is closed anyway.

**Actors**: `cpt-insightspec-actor-tenant-admin`, `cpt-insightspec-actor-oidc-provider`

### 5.15 Internal Endpoint Reachability

#### Two Listeners, Network Scopes, Credentials Everywhere

- [x] `p1` - **ID**: `cpt-insightspec-fr-auth-internal-reachability`

`/internal/authz` turns a stolen cookie value into a signed JWT, so its reachability is layered:

1. **Edge**: `/internal/*` never routes through the gateway (generated 404) and the ingress has a single backend -- the gateway -- so nothing external can name the authenticator.
2. **Network**: the authenticator **MUST** serve two listeners with different scopes. Main port (`/auth/*`, JWKS, `/internal/authz`): NetworkPolicy admits ingress from gateway pods only. Token port (`POST /internal/token` only): ingress from application-namespace service pods.
3. **Credential**: each endpoint still authenticates its caller regardless of network position -- `/internal/authz` requires a live session cookie; `/internal/token` requires a valid registry assertion. **Network position is never authentication.**

Compose/dev has no NetworkPolicies; acceptable -- layers 1 and 3 still hold there.

**Rationale**: Defense in depth around the one endpoint that exchanges a bearer credential for a signed one.

**Actors**: `cpt-insightspec-actor-nginx-gateway`, `cpt-insightspec-actor-downstream-service`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Session Lookup Latency

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-auth-exchange-p95`

The `/internal/authz` exchange (token mapping + session + JWT reads) **MUST** complete within 5 ms p95 under normal load, keeping total gateway overhead comfortably inside the 15 ms p95 budget the deleted gateway spec carried.

**Threshold**: 5 ms p95 for the exchange; two Redis reads on the hot path.

#### Session TTL Bounds

- [x] `p1` - **ID**: `cpt-insightspec-nfr-auth-session-ttl`

Session TTL and absolute lifetime **MUST** be configurable via Helm values without code change. Defaults: 600 s TTL (reasonable range 300-600 s), 8 h absolute cap.

**Threshold**: Operator can set both knobs via Helm values; they take effect on rolling restart.

#### Cookie Hardening

- [x] `p1` - **ID**: `cpt-insightspec-nfr-auth-cookie-attrs`

Every session cookie response **MUST** include `__Host-` prefix, `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, and no `Domain` attribute. A code path that would set a session cookie without all of these **MUST** fail closed.

**Threshold**: 100% of session-cookie responses match the attribute set.

#### Audit of Auth Events

- [x] `p1` - **ID**: `cpt-insightspec-nfr-auth-audit`

Every login, logout, session refresh, session revocation, back-channel logout, IdP-refresh verdict (`invalid_grant` kill), service-token issuance, and bootstrap-admin creation **MUST** emit an audit event consumed by the Audit Service.

**Threshold**: 100% coverage of auth events.

#### Rate Limiting on `/auth/*` (Layer 2)

- [x] `p1` - **ID**: `cpt-insightspec-nfr-auth-rate-limit`

The gateway carries a coarse per-IP flood guard (layer 1). The authenticator **MUST** enforce the precise layer: a Redis token bucket keyed by session/user (not IP -- corporate NAT makes per-IP limits at the precise layer wrong), and a global cap on concurrent live `asm:login_state:*` entries (default 1000 per pod) rejecting excess `/auth/login` with 429 before any Redis write.

**Threshold**: under a sustained login flood, login-state entries stay at or below the cap and CPU is bounded.

#### Fail Closed on Redis

- [x] `p1` - **ID**: `cpt-insightspec-nfr-auth-fail-closed`

If Redis is unreachable, `/internal/authz` and `/auth/*` mutations **MUST** fail (401/503) and the readiness probe **MUST** fail. No local session cache, no degraded mode.

**Threshold**: Zero exchanges served without a live Redis session read.

### 6.2 NFR Exclusions

- **HTTPS enforcement / HSTS**: owned by the nginx gateway, the component all traffic crosses (see [Gateway DESIGN](../gateway/DESIGN.md)). The authenticator is plain HTTP behind it, network-scoped per 5.15.
- **Per-route rate limiting on `/api/*`**: gateway `limit_req` (coarse) plus per-service middleware; not an authenticator concern.
- **Gateway latency budget**: the end-to-end 15 ms p95 budget is allocated across gateway + exchange; the authenticator's share is pinned by `cpt-insightspec-nfr-auth-exchange-p95`.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Auth API

- [x] `p1` - **ID**: `cpt-insightspec-interface-auth-api`

**Type**: REST API

**Stability**: stable

**Endpoints** (main listener unless noted):

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/auth/login` | Start OIDC flow; 302 to IdP. |
| GET | `/auth/callback` | OIDC callback; creates session + linked JWT; sets cookie; 302 to SPA. |
| POST | `/auth/refresh` | Rotate cookie, extend session TTL; return `{expires_at, refresh_at}`. |
| POST | `/auth/logout` | Revoke current session; clear cookie; return RP-logout URL. |
| GET | `/auth/me` | Current user, tenants, plus `{expires_at, refresh_at}`. |
| GET | `/auth/sessions` | List active sessions for current user. |
| DELETE | `/auth/sessions/{id}` | Revoke a specific session. |
| DELETE | `/auth/sessions` | Revoke all sessions of current user. Admin/service variant revokes by user id (gateway-JWT authenticated). |
| POST | `/auth/oidc/back-channel-logout` | Receive IdP back-channel logout tokens. |
| GET | `/auth/csrf` | Issue CSRF token bound to current session. |
| GET | `/internal/authz` | Cookie-to-JWT exchange for the gateway `auth_request` (never routed externally). |
| GET | `/.well-known/jwks.json` | Public keys for JWT verification. |
| POST | `/internal/token` | Service-token issuance (dedicated token listener). |

All endpoints are registered through the toolkit operation builder and land in the generated OpenAPI document, which is the machine-checkable form of the gateway subrequest contract.

### 7.2 External Integration Contracts

#### Gateway JWT Claim Contract

- [x] `p1` - **ID**: `cpt-insightspec-contract-auth-gateway-jwt`

**Direction**: defined and minted by the authenticator, consumed by every downstream service.

**Format**: signed JWT, ES256 (decided — see [DESIGN section 5](./DESIGN.md#resolved-step-07-es256-for-the-gateway-jwt)).

**Claims**: `iss`, `aud`, `sub`, `iat`, `exp`, `jti` plus `tenant_id`, `roles`, `sub_type`, `sid` -- exactly as specified in 5.6.

**Compatibility**: Additive custom claims only without a major version. The permissions service later changes claim *values* (roles), never the contract shape.

#### Authz Exchange Contract

- [x] `p1` - **ID**: `cpt-insightspec-contract-auth-authz-exchange`

**Direction**: provided to the nginx gateway.

**Protocol/Format**: HTTP subrequest to `GET /internal/authz`; 200 with `X-Gateway-Jwt: Bearer <jwt>` response header; 401 deny; `Cache-Control: max-age = min(authz_cache_max_age, jwt_exp - now - 60 s)` on 200, `no-store` on non-200.

**Compatibility**: Path, header name, and caching semantics are a versioned contract between the two artifacts; covered by an e2e test (gateway risk R8).

#### JWKS Distribution Contract

- [x] `p1` - **ID**: `cpt-insightspec-contract-auth-jwks-url`

**Direction**: configuration -- each downstream service is given the JWKS URL.

**Mechanism**: Helm value / env `GATEWAY_JWKS_URL` pointing at `/.well-known/jwks.json` through the gateway. Services fetch on startup, cache, refetch on unknown `kid`.

**Compatibility**: URL stable across minor releases; RFC 7517 schema.

#### OIDC Provider Contract

- [x] `p1` - **ID**: `cpt-insightspec-contract-auth-oidc`

**Direction**: required from customer.

**Protocol**: OIDC Authorization Code + PKCE; RP-initiated logout; back-channel logout (optional but recommended); **refresh-token issuance to this client** (some IdPs require the `offline_access` scope). When refresh tokens are not granted, `authenticator.idp.no_refresh_token_policy` governs session lifetime.

**Compatibility**: Standard OIDC.

#### Service Registry Contract

- [x] `p1` - **ID**: `cpt-insightspec-contract-auth-service-registry`

**Direction**: required from operators (gitops-reviewable config).

**Protocol/Format**: registry entries mapping service name to public key(s) (for RFC 7523 assertion verification), allowed extra roles, and tenant-scoping permission. Public keys are not secrets; the registry lives in reviewable configuration.

**Compatibility**: Additive; key rotation ships n+1 alongside n.

#### Authenticator SDK

- [x] `p2` - **ID**: `cpt-insightspec-contract-auth-sdk`

**Direction**: provided to internal consumers (e.g. the future permissions service).

**Protocol/Format**: `authenticator-sdk` crate -- the inter-gear contract trait (session revoke, introspection as needed), request/response models, optional typed error projection over canonical errors. Consumers depend on the SDK only, never on the implementation crate.

**Compatibility**: SemVer on the crate; additive trait evolution.

## 8. Use Cases

### 8.1 Browser Session Lifecycle

#### Login

- [x] `p1` - **ID**: `cpt-insightspec-usecase-auth-login`

**Actor**: `cpt-insightspec-actor-browser-user`

**Preconditions**: SPA loaded; no valid session cookie.

**Main Flow**:
1. SPA calls a protected API; the gateway's exchange returns 401 with a login URL.
2. Browser requests `/auth/login` (plain proxy through the gateway); authenticator stores `state`, `nonce`, PKCE verifier; redirects to IdP.
3. User authenticates at IdP; IdP redirects to `/auth/callback` with the code.
4. Authenticator validates `state`, exchanges the code (PKCE), validates the ID token; stores the IdP refresh token for background refresh.
5. Authenticator resolves person and tenant memberships via Identity Service; fetches access-control claims (default roles until the permissions service exists).
6. Authenticator creates the session (stable `session_id`, UUIDv7), the token mapping, **and mints the linked JWT** -- one pipeline.
7. Authenticator sets the session cookie and redirects to the SPA's original target.

**Postconditions**: Browser holds an opaque cookie; Redis holds session record, token mapping, linked JWT, index entries, and an IdP-refresh due entry. Audit event recorded.

**Alternative Flows**:
- **State or nonce mismatch**: 400, no session created.
- **Person not found and table not empty**: 403; audit records the failed login.
- **Person table empty and bootstrap enabled**: first-admin bootstrap per `cpt-insightspec-fr-auth-bootstrap`.

#### API Request (Cookie In, JWT Out)

- [x] `p1` - **ID**: `cpt-insightspec-usecase-auth-exchange`

**Actor**: `cpt-insightspec-actor-nginx-gateway`

**Preconditions**: Live session; gateway exchange-cache miss for this session token.

**Main Flow**:
1. Browser sends `GET /api/...` with the session cookie and nothing else.
2. Gateway subrequests `GET /internal/authz`.
3. Authenticator resolves token to session; JWT age under the reissue threshold: returns 200 + `X-Gateway-Jwt` + `Cache-Control: max-age=...`.
4. Gateway caches the exchange, strips the cookie, injects `Authorization: Bearer <jwt>` upstream.
5. Downstream service verifies the signature via JWKS and authorizes from claims.

**Postconditions**: Upstream saw only the JWT; the gateway serves subsequent requests for this session from its cache within max-age.

**Alternative Flows**:
- **No/expired session**: 401 with `no-store`; gateway returns 401 with the login URL, no upstream call.
- **JWT past reissue age**: authenticator rebuilds claims, signs fresh, `SET NX` stampede-safe, returns the canonical JWT.

#### Log Out Everywhere

- [x] `p1` - **ID**: `cpt-insightspec-usecase-auth-logout-everywhere`

**Actor**: `cpt-insightspec-actor-browser-user`

**Main Flow**:
1. User triggers "log out everywhere"; SPA calls `DELETE /auth/sessions`.
2. Authenticator enumerates the per-user index and, in one pipeline, deletes every session record, linked JWT, token mapping, sid-index entry, and refresh-schedule entry; clears the current cookie.

**Postconditions**: Every device's next exchange returns 401 (gateway-side within cache max-age, at most 30 s by default); in-flight JWTs die within their `exp` (at most 300 s). Audit events recorded per session.

**Alternative Flows**:
- **Admin- or service-initiated**: same operation against a target user via the gateway-JWT-authenticated admin surface (permission check enforced).

#### IdP Refresh Kill Path

- [x] `p1` - **ID**: `cpt-insightspec-usecase-auth-idp-refresh-kill`

**Actor**: `cpt-insightspec-actor-oidc-provider`

**Preconditions**: User disabled/revoked at the IdP; linked session still live; IdP has no back-channel logout.

**Main Flow**:
1. The refresher leader pops the session from `asm:idp_refresh_due`.
2. Refresh attempt returns `invalid_grant` (definitive).
3. Every session linked to that grant is revoked through the standard pipeline.

**Postconditions**: User is back at `/auth/login` within one session TTL. Audit + `invalid_grant` counter incremented.

**Alternative Flows**:
- **Transient failure (timeout, 5xx, 429)**: retry with backoff, honoring `Retry-After`; nobody is logged out by a blip; consecutive-failure gauge rises for alerting.

### 8.2 Service-to-Service Authentication

#### Service Token Issuance

- [x] `p1` - **ID**: `cpt-insightspec-usecase-auth-service-token`

**Actor**: `cpt-insightspec-actor-downstream-service`

**Preconditions**: Service registered (public key in the registry); background job needs to call another service.

**Main Flow**:
1. Service signs an RFC 7523 assertion and POSTs it to `/internal/token` (token listener).
2. Authenticator validates against the registry, replay-guards `jti`, audits the issuance.
3. Response: gateway JWT with `sub = service:<name>`, registry-allowed roles, TTL 300 s.
4. Service caches the token and re-requests before expiry.

**Postconditions**: The callee verifies the token through the same JWKS path as user traffic.

**Alternative Flows**:
- **Unknown service / bad signature / replayed jti**: 401, audited.

## 9. Acceptance Criteria

- [x] `cpt-insightspec-fr-auth-oidc-login`, `cpt-insightspec-fr-auth-session-cookie`: After login, no IdP token is present in anything delivered to the browser; the only auth artifact is the opaque `__Host-sid` cookie with the full attribute set and `Max-Age` matching the configured TTL.
- [x] `cpt-insightspec-fr-auth-session-model`: The JWT `sid` claim and all Redis session keys are unchanged across any number of cookie rotations within one session.
- [x] `cpt-insightspec-fr-auth-linked-jwt`, `cpt-insightspec-fr-auth-authz-exchange`: Every 200 from `/internal/authz` carries a JWT with at least 60 s of remaining validity and a `Cache-Control: max-age` no greater than `authz_cache_max_age`; every non-200 carries `no-store`.
- [x] `cpt-insightspec-fr-auth-session-revoke`: After "revoke all", every device returns 401 within one gateway-JWT TTL (at most 300 s); at the authenticator itself, immediately.
- [x] `cpt-insightspec-fr-auth-idp-refresh`: With the fake IdP's revoke control hook fired, all linked sessions die on the next scheduled refresh; with its outage hook fired, no session dies.
- [x] `cpt-insightspec-fr-auth-service-tokens`: A registered service obtains a `sub = service:<name>` JWT verifiable via the same JWKS; an unregistered caller gets 401.
- [ ] `cpt-insightspec-fr-auth-bootstrap`: On an empty persons table with bootstrap enabled, the first IdP login creates a universe admin and emits the audit event; the second login does not.
- [x] `cpt-insightspec-fr-auth-logout`, `cpt-insightspec-fr-auth-csrf`: Local, RP-initiated, and back-channel logout all converge on the same revoke pipeline; state-changing `/auth/*` requests without CSRF token or matching `Origin` are rejected 403.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Redis | Session records, token mappings, linked JWTs, indexes, refresh schedule, locks | `p1` |
| Customer OIDC provider | Authentication (code + PKCE), refresh tokens, RP-initiated + back-channel logout | `p1` |
| Identity Service | Map IdP `sub` to internal `person_id` and tenant memberships | `p1` |
| Audit Service | Sink for auth events | `p1` |
| Nginx gateway | Fronts all browser traffic; exchange caller; see [Gateway DESIGN](../gateway/DESIGN.md) | `p1` |
| Permissions service (future) | Access-control claims at login; session-revoke caller on grant change | `p3` |

## 11. Assumptions

- The customer OIDC provider supports authorization code + PKCE and RP-initiated logout; refresh-token issuance is expected (policy knob covers its absence); back-channel logout is optional (the refresher is the guaranteed deactivation path).
- The SPA and the gateway share one hostname; the SPA follows the refresh contract (server-supplied `refresh_at`, multi-tab leader election, 401 handling).
- Redis is deployed HA; session loss requires re-login for affected users -- acceptable.
- Dev and CI log in through a fake IdP exercising the same OIDC code path -- no dev-login bypass endpoint exists in the authenticator.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Redis outage | All users effectively logged out; logins blocked | HA Redis; fail closed; readiness probe |
| Authenticator down | No new exchanges; gateway fails closed with 503 + Retry-After | Stateless horizontal scaling; gateway exchange cache absorbs brief blips for already-cached sessions |
| Customer IdP withholds refresh tokens | Sessions capped at IdP access-token lifetime under `strict` policy | `no_refresh_token_policy` knob; documented in the OIDC provider contract |
| IdP outage during refresh wave | Sessions drift toward their cap | Fail-open-on-transport; backoff + `Retry-After`; consecutive-failure gauge alerts before mass logout |
| `logout_token` without `sid` widens blast radius | Back-channel logout becomes "log out everywhere" for that user | Runbook callout; log line on every `(iss, sub)`-only fallback |
| Bootstrap race on fresh install | First IdP-authenticated colleague wins universe admin | Empty-table-only window; loud audit; off switch; INSTALLER as production path |
| Gateway exchange cache staleness | Revocation reaches the gateway up to `authz_cache_max_age` late | Default 30 s, well inside the 300 s acceptance bound; set 0 for per-request checks |
| Registry misconfiguration | A service gets roles it should not have | Gitops review of every registry change; issuance audit trail |
