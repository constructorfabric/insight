---
status: proposed
date: 2026-04-28
---

# PRD -- BFF (Backend-for-Frontend) Service

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
  - [5.2 Session Cookie](#52-session-cookie)
  - [5.3 Session Refresh](#53-session-refresh)
  - [5.4 Session Store](#54-session-store)
  - [5.5 Gateway JWT (Downstream Token)](#55-gateway-jwt-downstream-token)
  - [5.6 Session Management](#56-session-management)
  - [5.7 Logout](#57-logout)
  - [5.8 CSRF Protection](#58-csrf-protection)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The BFF is the auth half of the Insight API Gateway. It runs the OIDC login flow against the customer's identity provider, holds the user session server-side, and exposes a small `/auth/*` API that the SPA uses to log in, refresh, list devices, and log out.

The browser only ever holds an opaque session cookie with a short TTL. The IdP token never reaches the browser. The session is extended only by an explicit `POST /auth/refresh` from the SPA -- never by passive activity.

Request forwarding to internal services and gateway JWT minting are owned by the sibling [Router](../router/PRD.md), not by the BFF.

### 1.2 Background / Problem Statement

The current frontend stores the OIDC access token in `localStorage`. Any XSS leaks every active token. Storage-bound tokens are also visible to browser extensions and developer tools.

Moving the token to an `HttpOnly`, `Secure`, `SameSite=Strict` cookie alone is not enough. The token in the cookie still grants long-lived access, cannot be revoked without a denylist, and exposes IdP claims to the browser tier. We want:

1. The browser to hold only an opaque session ID -- nothing usable if leaked off-host.
2. The session to be revocable instantly, including "log out everywhere" for one user.
3. Internal services to verify caller identity statelessly via a short-lived JWT signed by the gateway, not by the IdP.

### 1.3 Goals (Business Outcomes)

- Remove all IdP and access tokens from browser storage.
- Make sessions revocable per-session and per-user from a single store.
- Give every internal service a verifiable, short-lived identity claim per request.
- Keep the SPA simple -- no token handling code in the browser.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| OIDC token | Tokens issued by the customer's identity provider. The BFF stores only `id_token` (used as `id_token_hint` on RP-initiated logout); access and refresh tokens are received at login but not stored or used in v1. The browser never sees any of them. |
| Session cookie | Opaque, random session ID set on the browser by the BFF. Short, hard TTL. No claims, no meaning outside Redis. |
| Session record | Server-side object in Redis keyed by session ID. Holds user, tenant, IdP linkage (`iss`, `sub`, `sid`), `id_token` for logout hint, expiries, and CSRF token. |
| Session refresh | Explicit `POST /auth/refresh` call from the SPA that extends the session TTL. The session does **not** extend automatically on regular API calls. |
| User session index | Redis sorted set keyed by user ID. Members are session IDs, score is `expires_at`. Lets the BFF list active sessions and find expired ones in O(log N). |
| Gateway JWT | Short-lived EdDSA-signed JWT minted by the [Router](../router/PRD.md) for each upstream call. Verified by internal services via JWKS. |
| Downstream service | Any internal Insight service behind the gateway (Analytics API, Connector Manager, Identity Service, etc.). |

## 2. Actors

### 2.1 Human Actors

#### Browser User

**ID**: `cpt-insightspec-actor-browser-user`

**Role**: Any authenticated end user (Viewer, Analyst, Admin) accessing Insight through the SPA.
**Needs**: Log in, stay logged in across requests, log out, see their active sessions, revoke a session from another device.

#### Tenant Administrator

**ID**: `cpt-insightspec-actor-tenant-admin`

**Role**: Already defined in the parent backend PRD. Additionally needs to revoke any user's sessions (forced logout on role change, offboarding, suspected compromise).

### 2.2 System Actors

#### OIDC Provider

**ID**: `cpt-insightspec-actor-oidc-provider`

**Role**: Customer identity provider. Runs the authorization code + PKCE flow. May call back-channel logout.

#### Downstream Service

**ID**: `cpt-insightspec-actor-downstream-service`

**Role**: Any internal Insight service that receives the gateway JWT from the API gateway and authorizes the request based on its claims. The gateway is one service split into BFF and Router logic parts; from the downstream service's perspective the JWT comes from the gateway.

#### Redis

**ID**: `cpt-insightspec-actor-redis`

**Role**: Stores session records and the user-to-sessions index. The single source of truth for "who is logged in".

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Single deployment per Insight installation, fronted by the cluster ingress.
- Public hostname terminates TLS at the ingress; the BFF refuses requests received as plain HTTP.
- The BFF and the SPA are served from the same registrable domain so cookies are first-party.
- Stateless horizontally scalable -- all session state is in Redis.

## 4. Scope

### 4.1 In Scope

- OIDC authorization code + PKCE login flow as a confidential client.
- Opaque session cookie issuance with short hard TTL and hardened attributes.
- Explicit session refresh endpoint (`POST /auth/refresh`).
- Session record storage in Redis (BFF-prefixed keys) with a sorted-set per-user index keyed by `expires_at`.
- Session listing and revocation API (single, all-but-current, all).
- Logout: local, RP-initiated to OIDC provider, and OIDC back-channel logout receiver.
- CSRF defense for state-changing requests.
- Periodic cleanup of expired session entries from the user index.

### 4.2 Out of Scope

- Gateway JWT minting and signing -- owned by the [Router](../router/PRD.md).
- JWKS endpoint -- served by the Router.
- Reverse-proxying `/api/*` requests -- owned by the Router.
- Authorization decisions inside downstream services (each service still enforces RBAC and visibility).
- User registration, password management, MFA -- handled by the customer OIDC provider.
- License / role / scope claims in the gateway JWT -- not needed for v1; the contract carries the required JWT claims (`iss`, `aud`, `sub`, `iat`, `exp`, `jti`) plus `tid` and `sid`. See §5.5 / §7.2.
- Mobile or third-party API clients (v1 serves only the bundled SPA).

## 5. Functional Requirements

### 5.1 OIDC Login Flow

#### Authorization Code with PKCE

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-oidc-login`

The system **MUST** implement OIDC authorization code flow with PKCE as a confidential client. The BFF **MUST** generate `state`, `nonce`, and PKCE verifier per login attempt and validate them on callback. The browser **MUST NOT** receive or transmit the IdP code, ID token, or access token at any point.

The new `session_id` issued at the end of a successful callback **MUST** be generated server-side from a CSPRNG and **MUST NOT** be derived from, or equal to, any value present in the incoming request (cookies, headers, query). Any `__Host-sid` cookie present on the `/auth/callback` request **MUST** be ignored; if its value maps to a live session in Redis, that session **MUST** be revoked before the new session is created. This prevents session-fixation where an attacker plants a known SID before the victim logs in.

**Rationale**: The whole point of this redesign -- IdP tokens never leave the server.

**Actors**: `cpt-insightspec-actor-browser-user`, `cpt-insightspec-actor-oidc-provider`

### 5.2 Session Cookie

#### Session Cookie Issuance

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-session-cookie`

After a successful OIDC callback, the system **MUST** issue an opaque session cookie with these attributes:

- `__Host-` prefix (forces host-only + Secure + Path=/).
- `HttpOnly`.
- `Secure`.
- `SameSite=Strict`.
- Random value with at least 128 bits of entropy.
- Short hard TTL. The TTL **MUST** be configurable; default is 120 seconds. The cookie `Max-Age` **MUST** match the session record TTL in Redis.
- The TTL **MUST NOT** be extended automatically by activity. Only an explicit `POST /auth/refresh` extends it (see 5.3).
- An absolute hard cap (e.g. 8h, configurable) **MUST** apply across refreshes -- once `created_at + max_lifetime` is reached, refresh fails and the user must log in again.

The cookie value **MUST** be opaque -- no claims, no JWT, no user-identifying data.

**Rationale**: Short TTL plus explicit refresh limits the window for stolen-cookie reuse and gives the SPA explicit control over session lifetime. The absolute cap forces re-authentication on a known schedule.

**Actors**: `cpt-insightspec-actor-browser-user`

### 5.3 Session Refresh

#### Explicit Session Refresh Endpoint

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-session-refresh`

The system **MUST** expose `POST /auth/refresh`. The cookie value is the `session_id` and **rotates** on every successful refresh. Behaviour:

1. **Stale cookie / no session in Redis** → 401, clear the cookie.
2. **Cookie value found in `bff:session:*`** (normal path):
   1. Generate a fresh `session_id` (`new_sid`) from a CSPRNG, ≥128 bits entropy.
   2. Compute `new_exp = min(now + session_ttl, absolute_expires_at)`.
   3. Compute `refresh_at = (new_exp − safety_margin) + jitter`, where `jitter` is a uniform random offset in the range `[−jitter_window/2, +jitter_window/2]`. `safety_margin` defaults to 30 s. `jitter_window` defaults to 10 s.
   4. **Atomically** rotate the session: write the grace mapping `bff:swap:{old_sid} → new_sid` with a TTL of `grace_ms` (default `250 ms`); rename `bff:session:{old_sid}` to `bff:session:{new_sid}` and update `expires_at` + Redis TTL to `new_exp`; replace the user-index ZSET entry; replace the IdP-sid index entry; delete `router:jwt_cache:{old_sid}`. All of this in one batch.
   5. Re-issue the session cookie with the new `session_id` and `Max-Age = new_exp − now`.
   6. Return `200 {expires_at, refresh_at}`.
3. **Cookie value found in `bff:swap:*` (grace path)**:
   1. Resolve the swap to `new_sid`. The session is already current; do **not** rotate again.
   2. Read `expires_at` from `bff:session:{new_sid}`.
   3. Compute a freshly jittered `refresh_at`.
   4. Set the cookie to `new_sid` and return `200 {expires_at, refresh_at}`.

The grace window (`grace_ms`, default 250) **MUST** be small. Outside it, the old `session_id` is unrecoverable: 401 + clear cookie.

The system **MUST NOT** extend the session on any other endpoint or proxied API call. Regular `/api/*` traffic does not slide the TTL and does **not** rotate the cookie. A stale cookie on `/api/*` returns 401 immediately — the SPA must call `/auth/refresh` first.

The system **MUST NOT** call the IdP refresh-token endpoint as part of `/auth/refresh`. v1 does not store or use IdP access/refresh tokens after login. Hard-cap behaviour is delegated to the Redis TTL: once `new_exp` reaches `absolute_expires_at`, the next `EXPIREAT` either keeps the key alive briefly or evicts it; the user is then forced to log in again.

`GET /auth/me` **MUST** return the same `{expires_at, refresh_at}` fields (with a fresh `refresh_at` jitter) so the SPA can prime its refresh timer at page load.

**SPA contract.** The SPA **MUST** coordinate `/auth/refresh` calls across browser tabs of the same session, so only one tab fires the refresh per window. The recommended mechanism is `BroadcastChannel` with a `localStorage` fallback: a leader tab calls `/auth/refresh`, broadcasts the result, follower tabs read it without firing their own request. The SPA **MUST** use the server-supplied `refresh_at` (which is already jittered) as the scheduling target; an additional client-side jitter is permitted but not required.

**Rationale**: Cookie rotation detects token theft -- two parties holding the same cookie eventually diverge, and the one out of sync hits 401. The grace window absorbs benign races (page reload mid-refresh, parallel refresh from siblings during the brief leader-election window). Server-side jitter prevents an attacker from aligning a forged `/auth/refresh` to the predictable grace window. Multi-tab coordination eliminates the most common legit cause of parallel refreshes.

**Actors**: `cpt-insightspec-actor-browser-user`

### 5.4 Session Store

#### Redis-Backed Session Storage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-session-store`

The system **MUST**:

1. **Persist sessions** -- record every active session server-side with all fields needed to validate, refresh, and revoke it (user, tenant, IdP linkage, timestamps, hard cap, CSRF token, request fingerprint). The store is Redis; key family `bff:session:*`.
2. **Maintain a per-user session index** that lets the BFF look up every active session for a given user in sub-linear time. Used by the "list my devices" and "log out everywhere" flows. Key family `bff:user_sessions:*`.
3. **Maintain an IdP-sid lookup** that resolves an `(iss, idp_sid)` pair (carried in OIDC back-channel logout tokens) to the matching local session(s). Key family `bff:sid_index:*`.
4. **Make create / refresh / revoke atomic** -- a partial failure **MUST NOT** leave the session record and the indexes out of sync.
5. **Run a periodic janitor** that removes expired entries from the user-session index and emits a metric on any drift between the index and the underlying session records.

The exact Redis schema (field list, sorted-set scoring, atomicity mechanism, janitor interval) is specified in [DESIGN §3.7](./DESIGN.md#37-database-schemas--tables). The PRD only states *what* must hold; *how* it is implemented is the DESIGN's job.

**Rationale**: Server-side storage is what makes sessions revocable. The per-user index is what makes "list devices" and "revoke all" fast. The IdP-sid index is what makes back-channel logout work. Atomicity prevents zombie sessions. The janitor keeps the index honest.

**Actors**: `cpt-insightspec-actor-redis`

### 5.5 Gateway JWT (Downstream Token)

#### Gateway JWT Claim Contract

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-gateway-jwt`

The gateway JWT is **minted by the Router** (see [Router PRD §5.2](../router/PRD.md#52-gateway-jwt-mint-and-cache)). The BFF defines the **claim contract** that the Router fills.

The gateway JWT **MUST** be signed with **EdDSA (Ed25519)** and **MUST** carry exactly the following claims, separated as required JWT claims and Insight-specific custom claims:

**Required JWT claims (RFC 7519)**:

- `iss` -- gateway issuer URL.
- `aud` -- `internal-services`.
- `sub` -- internal `user_id`.
- `iat` -- issued at (epoch seconds).
- `exp` -- `iat + 60..300` (hard bounds).
- `jti` -- UUID v7 for traceability.

**Insight custom claims**:

- `tid` -- `tenant_id`.
- `sid` -- BFF session ID (opaque, useful for tracing only).

**Out of scope for v1**: license tier (`lic`), roles, scopes. Authorization is performed inside each downstream service against its own data. These claims may be added in a later major version of the contract; until then, downstream services **MUST NOT** rely on them being present.

The Router **MUST** publish the public verification key at `/.well-known/jwks.json` on the gateway. **JWKS distribution to downstream services**: each downstream service is configured (Helm value `gateway.jwks_url`, env `GATEWAY_JWKS_URL`) with the absolute URL of the JWKS endpoint. Services fetch and cache the key set at startup and re-fetch on unknown `kid`. There is no service discovery; the URL is explicit.

Downstream services **MUST** verify the signature, `iss`, `aud`, and `exp` against the cached JWKS -- no shared secrets.

**Rationale**: Short TTL kills the need for a JWT denylist. EdDSA signatures are small and fast to verify. Explicit JWKS URL avoids guessing or service-mesh dependencies.

**Actors**: `cpt-insightspec-actor-downstream-service`

### 5.6 Session Management

#### List Active Sessions

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-session-list`

The system **MUST** expose an authenticated endpoint that returns the active sessions for the calling user (created_at, expires_at, user_agent, ip, current=true/false). The endpoint **MUST** read from the user-sessions sorted set with `ZRANGEBYSCORE`, returning only entries with score > now.

**Actors**: `cpt-insightspec-actor-browser-user`

#### Revoke Sessions

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-session-revoke`

The system **MUST** support three revocation operations:

1. Revoke the current session (logout).
2. Revoke a specific other session by ID.
3. Revoke all sessions for a user (self "log out everywhere", or admin-initiated).

Each operation **MUST** delete the session record(s) and remove them from `bff:user_sessions:{user_id}` (`ZREM`) atomically, and (when feasible) call the OIDC provider's RP-initiated logout. After revocation, any in-flight gateway JWT **MUST** become invalid within one JWT TTL (≤ 300 s).

**Rationale**: Instant revocation is the main reason to keep the session opaque + server-side. Without "all-sessions" revocation, offboarding and compromise response are broken.

**Actors**: `cpt-insightspec-actor-browser-user`, `cpt-insightspec-actor-tenant-admin`

### 5.7 Logout

#### Logout (Local, RP-Initiated, Back-Channel)

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-logout`

The system **MUST** provide `POST /auth/logout` that revokes the current session, clears the cookie (`Max-Age=0`), and redirects (or returns a redirect URL) to the OIDC `end_session_endpoint` for RP-initiated logout.

The system **MUST** accept OIDC back-channel logout tokens at a dedicated endpoint, validate the `logout_token` per spec, locate sessions by `(iss, sid)` (direct lookup via `bff:sid_index:*`) or `(iss, sub)` (resolve `sub` to internal `user_id` via Identity Service, then walk `bff:user_sessions:{user_id}`), and revoke them.

The system **MUST** protect the back-channel endpoint against replay: every accepted `logout_token` **MUST** be recorded by `(iss, jti)` with a TTL of at least `iat + max_clock_skew`, and any subsequent delivery of the same `(iss, jti)` **MUST** short-circuit to a successful response without performing another revoke.

The system **MUST** document and accept that a `logout_token` carrying only `sub` (no `sid`) will revoke every active session for that user across all browsers ("log out everywhere"). This is OIDC-spec-compliant fallback behaviour, but operators **MUST** be informed in the runbook so a misconfigured IdP does not silently widen blast radius.

**Rationale**: Without back-channel logout, IdP-side session termination does not propagate. Without RP-initiated logout, users stay signed in to the IdP after pressing "log out". Without `jti` replay protection, a captured valid `logout_token` is replayable for as long as its signature verifies, enabling repeated forced revocations / DoS for reconnecting users.

**Actors**: `cpt-insightspec-actor-oidc-provider`, `cpt-insightspec-actor-browser-user`

### 5.8 CSRF Protection

#### CSRF Defense

- [ ] `p1` - **ID**: `cpt-insightspec-fr-bff-csrf`

For state-changing methods (POST, PUT, PATCH, DELETE) on `/auth/*`, the system **MUST** require either:

1. A double-submit CSRF token sent in `X-CSRF-Token` matching a value bound to the session, or
2. A verified `Origin` header matching the configured SPA origin.

`SameSite=Strict` is the primary defense; this requirement is the second line.

**Rationale**: `SameSite=Strict` mitigates most CSRF, but defense in depth is cheap and protects against same-site-but-different-path attack vectors.

**Actors**: `cpt-insightspec-actor-browser-user`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### HTTPS Enforcement

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-bff-https-only`

The system **MUST** reject all plain-HTTP requests at the ingress. The system **MUST** set `Strict-Transport-Security: max-age=31536000; includeSubDomains; preload` on every response.

**Threshold**: Zero responses served over HTTP.

#### Session Lookup Latency

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-bff-session-lookup-p95`

Session validation against Redis **MUST** complete within 5 ms p95 under normal load.

**Threshold**: 5 ms p95 Redis read.

#### Session TTL Bounds

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-bff-session-ttl`

The session TTL **MUST** be configurable. Allowed range: 30 seconds to 1 hour. Default: 120 seconds. The absolute lifetime cap **MUST** be configurable; default 8 hours; minimum 1 hour; maximum 24 hours.

**Threshold**: Operator can set both knobs via Helm values without code change.

#### Gateway JWT Algorithm

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-bff-jwt-algorithm`

The gateway JWT **MUST** be signed with EdDSA (Ed25519). No other algorithm is permitted.

**Threshold**: 100% of issued JWTs use `alg: EdDSA`. Any other algorithm in JWKS or in a token is rejected by downstream services.

#### Cookie Hardening

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-bff-cookie-attrs`

Every session cookie response **MUST** include `__Host-` prefix, `HttpOnly`, `Secure`, `SameSite=Strict`, `Path=/`, and no `Domain` attribute. A request that would set a session cookie without all of these **MUST** fail closed.

**Threshold**: 100% of session-cookie responses match the attribute set.

#### Audit of Auth Events

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-bff-audit`

Every login, logout, session refresh, session revocation, and back-channel logout **MUST** emit an audit event consumed by the Audit Service (see parent PRD `cpt-insightspec-fr-be-audit-trail`).

**Threshold**: 100% coverage of auth events.

#### Rate Limiting on `/auth/*`

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-bff-rate-limit-auth`

The BFF **MUST** rate-limit `/auth/login`, `/auth/callback`, and `/auth/refresh` per source IP (token bucket). Defaults: `auth_rate_per_ip = 10 req/min`, `auth_burst_per_ip = 20`. The BFF **MUST** also enforce a global cap on concurrent active `bff:login_state:*` entries (default `1000` per pod) and reject new `/auth/login` requests with `429` once the cap is hit, to prevent Redis exhaustion via a flood of unfinished login attempts.

**Threshold**: under sustained 100 req/s/IP attack, login does not consume more than `1000` `bff:login_state:*` entries per pod and CPU is bounded.

**Rationale**: an attacker can otherwise flood `/auth/login` with concurrent `state` UUIDs, each writing a 5-minute Redis HASH, and exhaust Redis memory or BFF event-loop CPU.

### 6.2 NFR Exclusions

- **Per-route rate limiting on `/api/*`**: Handled by the surrounding ingress and per-service middleware. The BFF rate-limits only `/auth/*` per `cpt-insightspec-nfr-bff-rate-limit-auth`.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Auth API

- [ ] `p1` - **ID**: `cpt-insightspec-interface-bff-auth-api`

**Type**: REST API

**Stability**: stable

**Endpoints**:

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/auth/login` | Start OIDC flow; 302 to IdP. |
| GET | `/auth/callback` | OIDC callback; sets session cookie; 302 to SPA. |
| POST | `/auth/refresh` | Extend session TTL; re-issue cookie; return `{expires_at, refresh_at}`. SPA schedules next call from `refresh_at`. |
| POST | `/auth/logout` | Revoke current session; clear cookie; return RP-logout URL. |
| GET | `/auth/me` | Return current user, tenant, plus `{expires_at, refresh_at}` so the SPA can prime its refresh timer at page load. |
| GET | `/auth/sessions` | List active sessions for current user. |
| DELETE | `/auth/sessions/{id}` | Revoke a specific session. |
| DELETE | `/auth/sessions` | Revoke all sessions of current user. |
| POST | `/auth/oidc/back-channel-logout` | Receive IdP back-channel logout tokens. |
| GET | `/auth/csrf` | Issue CSRF token bound to current session. |

JWKS publication and `/api/*` reverse proxy live on the [Router](../router/PRD.md), not on the BFF, but they share the same hostname and TLS endpoint.

### 7.2 External Integration Contracts

#### Gateway JWT Claim Contract

- [ ] `p1` - **ID**: `cpt-insightspec-contract-bff-gateway-jwt`

**Direction**: defined by BFF, minted by Router, consumed by every downstream service.

**Format**: EdDSA-signed JWT.

**Required JWT claims**: `iss`, `aud`, `sub`, `iat`, `exp`, `jti`.

**Insight custom claims**: `tid`, `sid`.

**Compatibility**: Additive custom claims only without a major version. Removing or changing the meaning of any claim requires a major version bump and coordinated rollout. License / role / scope claims are deliberately not present in v1.

#### JWKS Distribution Contract

- [ ] `p1` - **ID**: `cpt-insightspec-contract-bff-jwks-url`

**Direction**: configuration -- each downstream service is given the JWKS URL.

**Mechanism**: Helm value `gateway.jwks_url` and matching env `GATEWAY_JWKS_URL` injected into each downstream service. Default value points at the gateway's `/.well-known/jwks.json`. Services fetch on startup, cache 1 h, refetch on unknown `kid`.

**Compatibility**: URL is stable across minor releases. Schema follows RFC 7517 JWKS.

#### OIDC Provider Contract

- [ ] `p1` - **ID**: `cpt-insightspec-contract-bff-oidc`

**Direction**: required from customer.

**Protocol**: OIDC Authorization Code + PKCE; RP-initiated logout (`end_session_endpoint`); back-channel logout per OIDC spec. The BFF does not use IdP refresh tokens in v1.

**Compatibility**: Standard OIDC. Customer IdP must support all four.

## 8. Use Cases

#### Login

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-bff-login`

**Actor**: `cpt-insightspec-actor-browser-user`

**Preconditions**: SPA loaded; no valid session cookie.

**Main Flow**:
1. SPA calls a protected API; Router returns 401 with login URL.
2. Browser requests `/auth/login`. BFF generates `state`, `nonce`, PKCE verifier; stores them in `bff:login_state:{state}`; redirects to IdP.
3. User authenticates at IdP. IdP redirects browser to `/auth/callback` with code.
4. BFF validates `state`, exchanges code (with PKCE verifier) for tokens, validates ID token (`nonce`, `iss`, `aud`, signature, expiry).
5. BFF resolves IdP `sub` to internal user (Identity Service).
6. BFF creates `bff:session:{id}` and `ZADD bff:user_sessions:{user_id} {expires_at} {sid}`.
7. BFF sets the session cookie (short TTL) and redirects to the SPA's original target.

**Postconditions**: Browser holds a session cookie. Redis holds the session record and a sorted-set entry whose score is the session's `expires_at`. Audit event recorded.

**Alternative Flows**:
- **State or nonce mismatch**: BFF returns 400 and aborts. No session created.
- **IdP-resolved user not found**: BFF returns 403; audit event records the failed login.

#### Session Refresh

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-bff-refresh`

**Actor**: `cpt-insightspec-actor-browser-user`

**Preconditions**: Valid session cookie; current time before `absolute_expires_at`.

**Main Flow** (cookie rotation):
1. SPA leader tab calls `POST /auth/refresh` at the server-supplied (jittered) `refresh_at`.
2. BFF reads the cookie value `old_sid`, fetches `bff:session:{old_sid}`.
3. BFF generates a fresh `new_sid` (CSPRNG, ≥128 bits) and computes `new_exp = min(now + session_ttl, absolute_expires_at)`.
4. BFF runs a single MULTI/EXEC pipeline: write `bff:swap:{old_sid} → new_sid` (PX = `grace_ms`); rename `bff:session:{old_sid}` to `bff:session:{new_sid}` and update `expires_at` + Redis TTL; replace ZSET entry in `bff:user_sessions:{user_id}`; replace SET entry in `bff:sid_index:{iss}:{idp_sid}`; `DEL router:jwt_cache:{old_sid}`.
5. BFF re-issues the cookie with `new_sid` and `Max-Age = new_exp − now`. Body: `{expires_at: new_exp, refresh_at: jittered}`.
6. SPA leader broadcasts the result to follower tabs via `BroadcastChannel`/`localStorage`; followers do not fire their own refresh.

**Postconditions**: Cookie value rotated. Old `bff:session:{old_sid}` is gone; `bff:swap:{old_sid}` lives for `grace_ms`. ZSET score, key TTL, and IdP-sid index all reference the new `session_id`.

**Alternative Flows**:
- **Stale cookie within grace window**: BFF resolves `bff:swap:{old_sid} → new_sid`, returns `200` with `Set-Cookie new_sid`; **no** further rotation. Used when a sibling tab fires `/auth/refresh` between the leader's call and broadcast.
- **Stale cookie past grace window**: BFF returns `401` and clears the cookie. SPA redirects to `/auth/login`.
- **Past absolute cap**: `EXPIREAT` with a past timestamp evicts the key; the SPA's next request returns 401.

#### Log Out Everywhere

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-bff-logout-everywhere`

**Actor**: `cpt-insightspec-actor-browser-user`

**Main Flow**:
1. User triggers "log out everywhere" in the SPA.
2. SPA calls `DELETE /auth/sessions`.
3. BFF reads `bff:user_sessions:{user_id}` (`ZRANGEBYSCORE 0 +inf`), deletes every `bff:session:{sid}`, deletes the sorted set, instructs the Router to drop matching `router:jwt_cache:{sid}` entries.
4. BFF clears the current cookie. Audit events recorded for each session.

**Postconditions**: All cookies still in browsers point to nonexistent sessions; next request from any device returns 401. Within one gateway-JWT TTL, all in-flight requests fail.

**Alternative Flows**:
- **Admin-initiated**: Tenant Admin calls the same operation against a target user; permission check enforces admin scope.

#### Back-Channel Logout

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-bff-back-channel-logout`

**Actor**: `cpt-insightspec-actor-oidc-provider`

**Main Flow**:
1. IdP terminates a user's IdP session and POSTs a `logout_token` to `/auth/oidc/back-channel-logout`.
2. BFF validates the logout token (signature, `iss`, `aud`, `iat`, `events` claim).
3. BFF resolves `(iss, sid)` (or `sub`) to the matching session(s) and revokes them.

**Postconditions**: User's session is gone. Next browser request returns 401.

## 9. Acceptance Criteria

- [ ] `cpt-insightspec-fr-bff-oidc-login`, `cpt-insightspec-fr-bff-session-cookie`: After login, no IdP token is present in any cookie, header, or response body delivered to the browser. The only auth artifact in the browser is the opaque session cookie with `__Host-`, `HttpOnly`, `Secure`, `SameSite=Strict`, and a `Max-Age` matching the configured session TTL.
- [ ] `cpt-insightspec-fr-bff-session-refresh`: Without `/auth/refresh` calls, a session expires after `session_ttl` seconds regardless of `/api/*` activity. With periodic refresh, sessions live until the absolute cap.
- [ ] `cpt-insightspec-fr-bff-session-store`: Every active session appears at `bff:session:{id}` and as a member of `bff:user_sessions:{user_id}` with score = `expires_at`. A revocation removes both atomically. The janitor reduces drift to zero on each pass.
- [ ] `cpt-insightspec-fr-bff-session-list`, `cpt-insightspec-fr-bff-session-revoke`: A user can list their active sessions (only entries with score > now) and revoke one, all-but-current, or all. After "revoke all", every device returns 401 within one gateway-JWT TTL.
- [ ] `cpt-insightspec-fr-bff-gateway-jwt`: The Router-issued gateway JWT carries exactly `iss`, `aud`, `sub`, `iat`, `exp`, `jti`, `tid`, `sid` -- nothing else. Signed with EdDSA. Verifiable by downstream services against the JWKS URL configured in their Helm values.
- [ ] `cpt-insightspec-fr-bff-logout`: Local logout, RP-initiated logout, and back-channel logout all converge on session deletion plus user-index cleanup.
- [ ] `cpt-insightspec-fr-bff-csrf`: State-changing `/auth/*` requests without a valid CSRF token or matching `Origin` are rejected with 403.
- [ ] `cpt-insightspec-nfr-bff-https-only`: No HTTP request reaches application code; HSTS is set on every response.
- [ ] `cpt-insightspec-nfr-bff-session-ttl`: Operator can set `session_ttl` and `absolute_lifetime` via Helm values; values within the documented ranges take effect on rolling restart.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Redis | Session records and user-sessions sorted set | `p1` |
| Customer OIDC provider | Authentication (auth-code + PKCE), RP-initiated logout, back-channel logout | `p1` |
| Identity Service | Map IdP `sub` to internal `user_id` and tenant | `p1` |
| Audit Service | Sink for auth events | `p1` |
| Ingress / TLS terminator | HTTPS termination, HSTS, request routing | `p1` |
| Router (sibling) | Gateway JWT minting, JWKS, `/api/*` reverse proxy | `p1` |

## 11. Assumptions

- The customer OIDC provider supports authorization code + PKCE, RP-initiated logout, and back-channel logout. (Refresh-token support is not required in v1; the BFF does not call the IdP refresh endpoint.)
- The SPA and BFF are served from the same registrable domain (first-party cookies).
- The SPA schedules `/auth/refresh` from the server-supplied (jittered) `refresh_at`, coordinates a single leader tab via `BroadcastChannel` / `localStorage` so siblings do not fire parallel refreshes, and handles 401 by redirecting to `/auth/login`.
- Redis is deployed in HA mode; session loss requires re-login for affected users -- acceptable.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| SPA stops calling `/auth/refresh` (bug, throttled tab) | User is logged out mid-flow | Default TTL of 120 s with 60 s refresh cadence gives 60 s of slack; document recommended cadence; SPA must handle 401 cleanly |
| Redis outage | All users effectively logged out; logins blocked | HA Redis; degraded mode policy deferred to DESIGN |
| BFF on the auth-critical path | Single point of failure for all UI traffic | Stateless horizontal scaling; readiness probes; ingress retries |
| `SameSite=Strict` breaks deep links from external sites | User lands logged out when following email/Slack links | Documented behavior; fall back to `Lax` only if UX requires it |
| Janitor falls behind | `bff:user_sessions:*` accumulates expired entries | Metric on backlog size; alert when above threshold; pass interval is shorter than session TTL |
| Back-channel logout endpoint abuse | Spoofed logout tokens trigger session revocation | Strict OIDC `logout_token` validation: signature, `iss`, `aud`, `iat`, `events`. `jti` replay protection via `bff:logout_jti:{iss}:{jti}` SET-NX with TTL ≥ `iat + max_clock_skew`. |
| `logout_token` without `sid` widens blast radius | A misconfigured IdP that omits `sid` causes every back-channel logout to behave as "log out everywhere" for the named `sub` | Runbook callout; operator-facing log line on every `(iss, sub)`-only fallback so the pattern is detectable |
| User-sessions index drift | Index lists sessions that no longer exist (or vice versa) | Atomic ops via MULTI/EXEC pipeline; janitor reconciles |
| Multi-tab without coordination | Sibling tabs fire parallel `/auth/refresh`, one wins, the others miss the grace window and 401 → unnecessary user logout | SPA contract requires `BroadcastChannel`/`localStorage` leader election; grace window absorbs the rare residual race |
| Sophisticated attacker stays in sync with rotation | Stolen cookie remains usable as long as the attacker races every refresh | Server-side jitter on `refresh_at` raises timing-attack bar; rate limits on `/auth/*` raise volumetric noise; future option: revoke-all-on-stale-token-past-grace |
