---
status: proposed
date: 2026-04-28
---

# PRD -- API Gateway Router

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
  - [5.1 Session Validation](#51-session-validation)
  - [5.2 Gateway JWT Mint and Cache](#52-gateway-jwt-mint-and-cache)
  - [5.3 Route Resolution](#53-route-resolution)
  - [5.4 Reverse Proxy](#54-reverse-proxy)
  - [5.5 Header Rewriting](#55-header-rewriting)
  - [5.6 JWKS Publication](#56-jwks-publication)
  - [5.7 Config Management](#57-config-management)
  - [5.8 Signing Key Rotation](#58-signing-key-rotation)
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
- [13. Related Documents](#13-related-documents)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Router is the part of the API Gateway that handles every non-auth request. It validates the session cookie, mints (or fetches from cache) a short-lived gateway JWT carrying the caller's identity, then forwards the request to the right internal service over plain HTTP.

It also publishes the JWKS used by internal services to verify those JWTs, and owns the route table that maps URL prefixes to upstream services.

The Router is the sibling of the BFF inside the API Gateway. The BFF owns the session; the Router uses it. See [BFF PRD](../bff/PRD.md).

### 1.2 Background / Problem Statement

The BFF creates and stores user sessions, but it does not forward requests on its own. Requests to `/api/*` need to be:

1. Tied to a real user via the session cookie.
2. Stamped with a short-lived, signed identity token so internal services can authorize without trusting the network.
3. Routed to the correct service based on URL prefix.
4. Reconfigured without redeployment when a new service is added or a route changes.

Doing this in the BFF would mix browser session concerns with cluster routing concerns. Splitting them gives a small, focused, hot-path component (Router) and a larger session-aware component (BFF) with their own change cadences.

### 1.3 Goals (Business Outcomes)

- Add no more than 15 ms p95 latency between the browser and the internal service.
- Make every internal service receive a fresh, verifiable identity claim per request.
- Allow operators to add or change routes via config -- no code change, no full restart.
- Rotate signing keys without downtime.

### 1.4 Glossary

| Term | Definition |
|---|---|
| Session cookie | Opaque cookie issued by the BFF (`__Host-sid`). Read-only from the Router's point of view. |
| Session record | Server-side state in Redis, owned by the BFF. The Router only reads it. |
| Gateway JWT | Short-lived JWT signed by the Router and consumed by internal services. Same token described in the BFF DESIGN; ownership moves here. |
| Route | A mapping `path-prefix → upstream-service-base-url` plus per-route options. |
| Route table | The full set of routes loaded from config. |
| JWKS | Public keys served at `/.well-known/jwks.json` for downstream services to verify gateway JWTs. |

## 2. Actors

### 2.1 Human Actors

#### Operator

**ID**: `cpt-insightspec-actor-operator`

**Role**: Platform engineer who deploys and configures Insight on the customer cluster.
**Needs**: Add a new internal service to the route table, change a route's timeout, rotate signing keys -- all without writing Rust.

### 2.2 System Actors

#### BFF (sibling component)

**ID**: `cpt-insightspec-actor-bff`

**Role**: Owns the session record. The Router reads sessions through the shared session manager interface owned by the BFF.

#### Browser User

**ID**: `cpt-insightspec-actor-browser-user`

**Role**: Already defined in the BFF PRD. From the Router's perspective, the browser is the source of cookies and forwarded requests.

#### Downstream Service

**ID**: `cpt-insightspec-actor-downstream-service`

**Role**: Receiver of forwarded requests with the gateway JWT. Verifies the JWT against JWKS and applies its own RBAC.

#### Redis

**ID**: `cpt-insightspec-actor-redis`

**Role**: Read-only access to `bff:session:*`; read/write on `router:jwt_cache:*`. The Router never writes session records.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Same process and pod as the BFF -- the API Gateway is one binary with two modules. Single ingress entry, single TLS endpoint.
- Stateless. Any pod can serve any request. Hot path uses Redis only.
- Route config and signing keys are loaded from K8s ConfigMap and Secret on startup, then watched for changes.

## 4. Scope

### 4.1 In Scope

- Cookie-based session validation against Redis.
- Gateway JWT mint, signing (EdDSA), and per-session caching in Redis.
- Route table loading from ConfigMap.
- Hot reload of route config and signing keys without restart.
- HTTP reverse proxy (request and response streaming, including chunked + WebSocket upgrades).
- Header rewriting (strip browser-supplied auth, inject `Authorization: Bearer ...`, always strip and regenerate `X-Correlation-Id` as UUID v7).
- JWKS endpoint with key rotation overlap.
- Per-route timeout enforcement.
- Health and readiness probes for K8s.

### 4.2 Out of Scope

- OIDC handshake, session creation, session revocation, logout -- handled by the [BFF](../bff/PRD.md).
- Per-tenant rate limiting -- handled by the surrounding ingress and per-service middleware (see parent backend NFR `cpt-insightspec-nfr-be-rate-limiting`).
- M2M API -- future
- Public M2M API for services -- future
- Service discovery (Consul, mesh) -- routes are explicit, ConfigMap-driven.
- Authorization decisions (role / scope / tenant filtering) -- every downstream service does its own.

## 5. Functional Requirements

### 5.1 Session Validation

#### Cookie-Based Session Validation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-session-validate`

For every request matched by the route table, the system **MUST** read the session cookie and look up the BFF-owned key `bff:session:{id}` in Redis. The request **MUST** be rejected with 401 if the cookie is missing, malformed, expired, or not present in Redis. The cookie value **MUST NOT** be logged or echoed back in any response, header, or log line.

The system **MUST NOT** modify session state. All writes (refresh, revoke) belong to the BFF.

**Rationale**: The Router cannot mint a JWT for a non-user. It also cannot duplicate session lifecycle code -- that belongs to the BFF.

**Actors**: `cpt-insightspec-actor-browser-user`, `cpt-insightspec-actor-redis`

### 5.2 Gateway JWT Mint and Cache

#### EdDSA-Signed Gateway JWT Per Request

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-jwt-mint`

For every forwarded request, the system **MUST** attach a gateway JWT signed with the current EdDSA key. The JWT **MUST**:

- Carry exactly the claims defined in the contract: required JWT claims `iss`, `aud`, `sub`, `iat`, `exp`, `jti`; Insight custom claims `tid`, `sid`. No `lic` / `roles` / `scopes` in v1. See [BFF DESIGN §3.8](../bff/DESIGN.md#38-gateway-jwt-claim-contract).
- Have `exp - iat` between 60 and 300 seconds.

The system **MUST** cache the minted JWT in Redis (`router:jwt_cache:{session_id}`) with TTL = `min(60s, jwt_remaining)`. Cache hits **MUST** skip the signing step.

The cache **MUST** be invalidated by the BFF deleting `router:jwt_cache:{sid}` on shared Redis as part of the session-revoke MULTI/EXEC pipeline. The Router itself runs no subscriber and uses no event stream for this purpose. v1 has no other invalidation source -- the JWT carries only `sub`, `tid`, and `sid`, none of which change during an active session.

**Rationale**: A signed JWT per request is the zero-trust contract. Caching keeps mint cost low under load. Direct Redis DEL by the BFF makes revoke-driven invalidation a single Redis round-trip with no eventual-consistency window.

**Actors**: `cpt-insightspec-actor-downstream-service`, `cpt-insightspec-actor-redis`

### 5.3 Route Resolution

#### Longest-Prefix Route Match

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-route-resolve`

The system **MUST** resolve every incoming request path to one upstream service using a longest-prefix match against the route table. Unmatched paths **MUST** return 404 without contacting any upstream.

Each route entry **MUST** specify at minimum:

- `prefix` (path prefix to match).
- `upstream` (base URL of the internal service).
- `timeout_ms` (per-request timeout).
- `strip_prefix` (boolean).
- `websocket` (boolean).

**Rationale**: Explicit routes are simpler and easier to audit than service discovery for a fixed-deployment product.

**Actors**: `cpt-insightspec-actor-operator`

### 5.4 Reverse Proxy

#### Streaming Reverse Proxy

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-proxy`

The system **MUST** forward the matched request to the resolved upstream and stream the response back. Body streaming **MUST** be supported in both directions to keep memory bounded for large CSV exports and uploads. WebSocket upgrades **MUST** be supported on routes flagged `websocket: true`.

The per-route `timeout_ms` **MUST** be enforced on upstream connect, write, and idle read.

**Rationale**: Analytics exports and pipeline status streams need streaming. Browser-to-backend WebSockets are needed for live dashboard updates.

**Actors**: `cpt-insightspec-actor-downstream-service`

### 5.5 Header Rewriting

#### Strip + Inject on Forward

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-header-rewrite`

Before forwarding, the system **MUST** strip headers in two categories and pass everything else through:

**Hardcoded gateway-reserved (always stripped, then re-set by the gateway)**:
- `Authorization` -- replaced with `Bearer <gateway-jwt>`.
- `X-Correlation-Id` -- always stripped from the incoming request and regenerated as UUID v7 by the gateway. Client-supplied values **MUST NOT** be propagated; this prevents browser-supplied values from poisoning correlation logs across tenants.
- `X-Forwarded-For`, `X-Forwarded-Proto`, `X-Forwarded-Host` -- set by the gateway per RFC.
- Gateway-reserved cookies (`__Host-sid`, CSRF cookie) -- stripped from `Cookie` header.

**Operator-configured (stripped only)**: any header name listed in `defaults.strip_request_headers` in the route ConfigMap. Reserved gateway header names **MUST NOT** appear in this list (validation rejects the config).

Response headers **MUST** be passed through with no modification except for stripping any `Set-Cookie` that uses a reserved cookie name.

**Rationale**: Browser-supplied `Authorization` headers must never reach internal services -- only the gateway's signed JWT does. Operator-configurable strip list lets deployments harden header hygiene without code changes; security-critical strips stay hardcoded so a misconfig cannot expose them.

**Actors**: `cpt-insightspec-actor-downstream-service`

### 5.6 JWKS Publication

#### JWKS Endpoint

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-jwks`

The system **MUST** serve `GET /.well-known/jwks.json` returning the current and previous public verification keys with stable `kid` values. The response **MUST** include `Cache-Control: public, max-age=3600`.

**Rationale**: Internal services verify gateway JWTs using JWKS and cache the result. Stable `kid` values let them refresh on unknown `kid` only.

**Actors**: `cpt-insightspec-actor-downstream-service`

### 5.7 Config Management

#### ConfigMap Load and Validation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-config-load`

The system **MUST** load route configuration from a K8s ConfigMap on startup and validate it against a schema (unique prefixes, valid URLs, sane timeouts). Validation failure **MUST** prevent the service from becoming ready -- never start with a partially valid table.

#### Atomic Hot Reload

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-config-reload`

The system **MUST** detect ConfigMap changes and apply the new route table without restart. Reload **MUST** be atomic -- no request **MUST** see a half-applied table. If the new config fails validation, the running config **MUST** be retained and an alert emitted.

In-flight requests **MUST** continue to use the route they were matched against; only new requests use the updated table.

**Rationale**: Adding a new internal service should be a config push, not a redeploy. Failed reloads must never break a running gateway.

**Actors**: `cpt-insightspec-actor-operator`

### 5.8 Signing Key Rotation

#### Hot Key Rotation with Overlap

- [ ] `p1` - **ID**: `cpt-insightspec-fr-router-key-rotation`

The system **MUST** load EdDSA signing keys from a K8s Secret with at least `current` and optional `previous` entries. The system **MUST** detect Secret changes and apply them without restart. JWKS **MUST** publish both keys when both are present so downstream services can verify tokens minted under the old key during the overlap window.

The system **MUST** sign new JWTs only with `current`.

**Rationale**: Operators must be able to rotate keys without a deployment.

**Actors**: `cpt-insightspec-actor-operator`, `cpt-insightspec-actor-downstream-service`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Latency Budget

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-router-latency`

Router overhead per forwarded request (cookie validate + JWT mint-or-cache + proxy hop) **MUST** be ≤ 15 ms p95 under nominal load (1k rps per pod).

**Threshold**: 15 ms p95 added latency.

#### JWT Cache Hit Rate

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-router-cache-hit`

Under sustained traffic, the gateway-JWT cache hit rate **MUST** be ≥ 80%.

**Threshold**: ≥ 80% hits over a 5-minute window for any session active for more than 60 s.

#### Config Reload Time

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-router-reload-time`

A valid ConfigMap change **MUST** take effect within 30 s of being written.

**Threshold**: 30 s p95 from ConfigMap update to first request using new route.

#### Fail-Closed Behavior

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-router-fail-closed`

If Redis is unreachable, signing keys are missing, or the route table is empty, the Router **MUST** return 503 for `/api/*` and report not-ready to K8s. It **MUST NOT** serve requests with stale keys, no JWT, or guessed routes.

**Threshold**: Zero requests forwarded without a valid session and a valid signed JWT.

### 6.2 NFR Exclusions

- **Per-tenant rate limiting**: Inherited from parent backend NFR; ingress and per-service middleware handle it.
- **Distributed tracing**: Inherited as out-of-scope from the parent backend PRD; correlation_id only.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Reverse Proxy

- [ ] `p1` - **ID**: `cpt-insightspec-interface-router-proxy`

**Type**: HTTP reverse proxy

**Stability**: stable

**Description**: Any `/api/**` path matching a configured route is forwarded to its upstream with `Authorization: Bearer <gateway-jwt>` injected.

#### JWKS

- [ ] `p1` - **ID**: `cpt-insightspec-interface-router-jwks`

**Type**: REST endpoint

**Stability**: stable

**Description**: `GET /.well-known/jwks.json` -- public keys for gateway JWT verification. See [BFF DESIGN section 3.8](../bff/DESIGN.md) for the JWT schema.

### 7.2 External Integration Contracts

#### Gateway JWT Contract

- [ ] `p1` - **ID**: `cpt-insightspec-contract-router-gateway-jwt`

**Direction**: provided by Router, consumed by every downstream service.

**Format**: same schema as `cpt-insightspec-contract-bff-gateway-jwt`. Ownership of the contract moves from BFF to Router with this PRD.

#### Route Configuration Contract

- [ ] `p1` - **ID**: `cpt-insightspec-contract-router-config`

**Direction**: required from operator.

**Format**: YAML in a K8s ConfigMap. Schema validated on load and reload.

**Compatibility**: Additive fields permitted in any minor version. Removing or renaming a field requires a major version bump.

## 8. Use Cases

#### Forwarding an Authenticated Request

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-router-forward`

**Actor**: `cpt-insightspec-actor-browser-user`

**Preconditions**: Browser has a valid session cookie issued by the BFF.

**Main Flow**:
1. Browser sends `GET /api/analytics/...` with cookie.
2. Router matches the path prefix to an upstream.
3. Router validates the cookie against `bff:session:{id}` in Redis (read-only).
4. Router reads `router:jwt_cache:{sid}`. On miss, mints a new JWT and caches it.
5. Router rewrites headers and forwards to the upstream.
6. Router streams the response back.

**Postconditions**: Downstream service received a request with a fresh signed JWT. Browser got the response.

**Alternative Flows**:
- **No / bad / expired cookie**: 401, no upstream call.
- **Path matches no route**: 404, no upstream call.
- **Upstream timeout**: 504 with retry-after.

#### Adding a New Internal Service

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-router-add-route`

**Actor**: `cpt-insightspec-actor-operator`

**Main Flow**:
1. Operator edits the gateway ConfigMap, adds a new route entry pointing to the new service's ClusterIP URL.
2. Operator commits and applies via ArgoCD.
3. Router detects the ConfigMap change, validates the new table.
4. Router atomically swaps the active route table.
5. Next request to the new prefix is forwarded to the new service.

**Postconditions**: New route live, no pod restart, no in-flight request affected.

**Alternative Flows**:
- **Validation fails**: Old table stays active, alert fires, operator fixes the YAML.

#### Rotating Signing Keys

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-router-rotate-key`

**Actor**: `cpt-insightspec-actor-operator`

**Main Flow**:
1. Operator generates a new EdDSA key pair.
2. Operator updates the K8s Secret: previous current → `previous`, new key → `current`.
3. Router reloads keys; JWKS now serves both.
4. New JWTs are signed with the new key. Existing JWTs (≤300 s old) verify against `previous`.
5. After overlap window (≥ JWT max TTL + downstream JWKS cache TTL), operator removes `previous` from the Secret.

**Postconditions**: All gateway JWTs are signed with the new key; old key is gone.

## 9. Acceptance Criteria

- [ ] `cpt-insightspec-fr-router-session-validate`, `cpt-insightspec-fr-router-jwt-mint`: Every request reaching a downstream service carries a freshly-signed JWT verifiable against `/.well-known/jwks.json`. No request reaches downstream without a valid session.
- [ ] `cpt-insightspec-fr-router-route-resolve`, `cpt-insightspec-fr-router-proxy`: Adding a new route via ConfigMap and waiting ≤30 s makes the new service reachable through the gateway with no restart.
- [ ] `cpt-insightspec-fr-router-header-rewrite`: An incoming request with a forged `Authorization` header reaches downstream with that header replaced by the gateway JWT, never preserved.
- [ ] `cpt-insightspec-fr-router-jwks`, `cpt-insightspec-fr-router-key-rotation`: Key rotation completes with overlap; downstream services accept tokens minted by either key during the overlap window and only the new key after the previous key is removed.
- [ ] `cpt-insightspec-nfr-router-latency`: Load test shows ≤15 ms p95 router overhead at 1k rps per pod.
- [ ] `cpt-insightspec-nfr-router-fail-closed`: Killing Redis returns 503 with not-ready probe; never serves a request with no JWT or stale data.

## 10. Dependencies

| Dependency | Description | Criticality |
|---|---|---|
| BFF (sibling) | Owner of session creation/destruction; provides the session manager library used by the Router | `p1` |
| Redis | Session reads + JWT cache | `p1` |
| K8s ConfigMap | Route table source | `p1` |
| K8s Secret | Signing keys source | `p1` |
| Downstream services | Targets of forwarded requests | `p1` |

## 11. Assumptions

- BFF and Router are deployed in the same pod (same process, separate modules).
- All internal services trust the Router as the only legitimate issuer of gateway JWTs.
- Operator workflow is GitOps -- ConfigMap and Secret changes flow through Git → ArgoCD; no out-of-band kubectl edits.
- The route table is small (tens of entries), so longest-prefix lookup is in-memory and fast.

## 12. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Bad ConfigMap reload silently breaks routes | Some routes return 404 unexpectedly | Strict schema validation on load and reload; retain previous table on validation failure; emit alert |
| JWT cache stampede during signing-key rotation | Mint-rate spike when all caches are invalidated together | Stagger cache invalidation per session; signing is fast (EdDSA), but verify under load |
| Misconfigured `strip_prefix` exposes wrong path to upstream | Downstream service returns 404 or, worse, hits the wrong handler | Integration tests per route; alert on sustained 4xx from a route after change |
| Header rewrite bug leaks browser `Authorization` downstream | Internal service accepts a forged identity | Snapshot tests on outbound headers; fuzz tests with malicious cookie/header combinations |
| Redis blip blocks all `/api/*` traffic | Whole product unavailable | Inherits BFF mitigation: HA Redis and fail-closed behavior; no degraded-read mode (see BFF DD-BFF-06) |
| Operator deletes `current` without overlap | All gateway JWTs invalid until reload | Documented runbook; admission-controller-style validation on the Secret if feasible |
| Operator removes `previous` signing key before overlap window elapses | Cached JWTs signed with `previous` are rejected by downstream services that already refetched JWKS | Runbook enforces minimum overlap = `jwt_ttl + downstream_jwks_max_age` ≈ 65 min. Optional `router:jwt_cache:*` flush on rotation eliminates residue. See key-rotation flow in DESIGN §3.6. |
| WebSocket revocation lag | Sessions revoked while a WS is open continue receiving traffic up to `websocket_max_lifetime_seconds`. Default 1 h global ceiling can be too lax for high-sensitivity streams. | Per-route `websocket_max_lifetime_seconds` override in the route ConfigMap (see DESIGN §3.8); tighten globally via Helm; future revocation-triggered disconnect via shared Redis pub/sub if needed (see DD-ROUTER-07). |
| ConfigMap reload leaves WS connections on a removed route | Decommissioned upstream keeps receiving traffic via still-open WebSockets up to the lifetime cap | ConfigMap Watcher walks the open-WebSocket registry on reload and closes sockets whose matched route was removed or had its upstream changed. See DESIGN §3.6 Config reload. |

## 13. Related Documents

- [BFF PRD](../bff/PRD.md) -- session creation, OIDC, logout, CSRF, refresh
- [BFF DESIGN](../bff/DESIGN.md) -- session storage model, gateway JWT schema (section 3.8)
- [Backend PRD](../../specs/PRD.md) -- parent platform PRD
- [Backend DESIGN](../../specs/DESIGN.md) -- parent platform DESIGN
