---
status: proposed
date: 2026-04-28
---

# DESIGN -- API Gateway

- [ ] `p3` - **ID**: `cpt-insightspec-design-gw`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
- [2. Module Boundary](#2-module-boundary)
- [3. Shared Infrastructure](#3-shared-infrastructure)
- [4. Deployment Topology](#4-deployment-topology)
- [5. Cross-Cutting Concerns](#5-cross-cutting-concerns)
  - [5.1 Configuration Surface](#51-configuration-surface)
  - [5.2 Observability](#52-observability)
  - [5.3 Failure Handling](#53-failure-handling)
- [6. Traceability](#6-traceability)

<!-- /toc -->

---

## 1. Architecture Overview

The API Gateway is a single Rust binary built on cyberfabric-core ModKit. Two modules, one process, one TLS endpoint:

```mermaid
graph TB
    subgraph Browser
        SPA[React SPA]
    end

    subgraph Ingress
        ING[K8s Ingress<br/>TLS · HSTS]
    end

    subgraph GW["API Gateway pod (single binary)"]
        BFF["BFF module<br/>/auth/*"]
        ROUTER["Router module<br/>/api/* + JWKS"]
        SM["Session Manager (lib)<br/>BFF-owned, used by both"]
    end

    subgraph State
        RD[(Redis<br/>bff:* + router:*)]
        CM[K8s ConfigMap<br/>routes.yaml]
        SECRET[K8s Secret<br/>signing keys]
    end

    subgraph External
        OIDC[Customer OIDC Provider]
        ID[Identity Service]
        AU[Audit Service]
    end

    subgraph DS[Downstream Services]
        S1[Analytics API]
        S2[Connector Manager]
        S3[Identity Service API]
        SX[...]
    end

    SPA --> ING
    ING --> BFF
    ING --> ROUTER
    BFF --> SM
    ROUTER --> SM
    SM --> RD
    ROUTER --> CM
    ROUTER --> SECRET
    BFF <--> OIDC
    OIDC -.->|back-channel logout| BFF
    BFF --> ID
    BFF --> AU
    ROUTER -->|Bearer JWT| S1
    ROUTER -->|Bearer JWT| S2
    ROUTER -->|Bearer JWT| S3
    ROUTER -->|Bearer JWT| SX
    S1 -.JWKS.-> ROUTER
    S2 -.JWKS.-> ROUTER
    S3 -.JWKS.-> ROUTER
    SX -.JWKS.-> ROUTER
```

Detailed component models, sequence diagrams, and data models live in the module DESIGNs:

- [BFF DESIGN](./bff/DESIGN.md) -- session lifecycle, OIDC, Redis layout, gateway JWT contract.
- [Router DESIGN](./router/DESIGN.md) -- JWT mint, JWKS, route table, reverse proxy.

## 2. Module Boundary

| Concern | Owner | Notes |
|---|---|---|
| OIDC handshake | BFF | Router never talks to the IdP |
| Session create / refresh / revoke | BFF | All ops are HMGET + MULTI/EXEC pipelines; no Lua scripts |
| Cookie issue / clear | BFF | Router never sets cookies |
| CSRF token issue & verify on `/auth/*` | BFF | Router relies on `SameSite=Strict` for `/api/*` |
| IdP access-token refresh | _(not in v1)_ | Tokens not stored or refreshed; v1 never calls IdP-protected APIs on the user's behalf. |
| Cookie validation on `/api/*` | Router (read-only via shared session manager) | Calls into the BFF-owned library |
| Gateway JWT mint + sign | Router | EdDSA, claims defined in BFF DESIGN §3.8 |
| JWKS publication | Router | `/.well-known/jwks.json` |
| Reverse proxy `/api/*` | Router | Forwards with `Authorization: Bearer <jwt>` |
| Route table + hot reload | Router | ConfigMap-driven |
| Signing key store + rotation | Router | K8s Secret-driven |
| Session manager library | BFF | Used by Router as a Rust crate |

## 3. Shared Infrastructure

| Resource | Used by | Notes |
|---|---|---|
| Redis client | both | Single connection pool, multiplexed; `bff:*` keys vs `router:*` keys |
| Audit emitter | both | Single Redpanda producer; auth events from BFF, key-rotation and config-reload events from Router |
| Metrics registry | both | Single Prometheus endpoint at `/metrics`; metrics prefixed `bff_*` or `router_*` |
| Logger | both | Single structured-JSON logger with `correlation_id` |
| HTTP server | both | Single `axum` router; `/auth/*` to BFF, everything else to Router |
| Config | both | Helm values surface both modules' knobs in one place |

## 4. Deployment Topology

- [ ] `p3` - **ID**: `cpt-insightspec-topology-gw`

- One Helm chart, one Deployment, ≥2 replicas.
- Single ClusterIP Service; cluster Ingress is the only TLS terminator.
- Mounts: `K8s ConfigMap routes.yaml` (Router), `K8s Secret bff-signing-keys` (Router).
- Connects to Redis (cluster Service) and the K8s API for ConfigMap/Secret watch.
- Liveness probe: process up. Readiness probe: Redis reachable + signing keys loaded + non-empty route table + Identity Service reachable.

```mermaid
graph TB
    HELM[Helm chart<br/>api-gateway] --> DEP[Deployment<br/>≥2 replicas]
    DEP --> POD[Pod<br/>insight-api-gateway]
    POD --> BIN[Binary: insight-gateway<br/>BFF + Router]
    POD -. mount .-> CFG[ConfigMap routes.yaml]
    POD -. mount .-> KEYS[Secret bff-signing-keys]
    POD --> SVC[Service<br/>ClusterIP]
    SVC --> ING[Ingress<br/>HTTPS]
```

## 5. Cross-Cutting Concerns

### 5.1 Configuration Surface

Helm values that affect both modules:

| Value | Default | Description |
|---|---|---|
| `gateway.replicas` | 2 | Pod count |
| `gateway.image` | (chart) | Container image |
| `gateway.session_ttl_seconds` | 120 | Session cookie TTL (BFF) |
| `gateway.session_refresh_safety_margin_seconds` | 30 | Base offset for `refresh_at = expires_at − safety_margin` (BFF) |
| `gateway.refresh_jitter_seconds` | 10 | Total jitter window applied to `refresh_at`; uniform random ∈ ±half this value (BFF) |
| `gateway.refresh_grace_ms` | 250 | TTL of `bff:swap:{old_sid}`; window in which a just-rotated cookie can still resolve on `/auth/refresh` (BFF) |
| `gateway.session_absolute_lifetime_seconds` | 28800 | Hard cap (BFF) |
| `gateway.jwt_ttl_seconds` | 120 | Gateway JWT TTL (Router); must be ≤300 |
| `gateway.websocket_max_lifetime_seconds` | 3600 | Global hard cap on WebSocket connection lifetime (Router). Per-route override available in `routes.yaml`. Bounds post-revoke staleness, see [Router DD-ROUTER-07](./router/DESIGN.md#dd-router-07-websocket-jwt-frozen-at-upgrade-time-bounded-by-max-lifetime). |
| `gateway.auth_rate_per_ip` | 10 | `/auth/*` token-bucket rate per source IP, requests/min (BFF) |
| `gateway.auth_burst_per_ip` | 20 | `/auth/*` token-bucket burst per source IP (BFF) |
| `gateway.auth_login_state_max` | 1000 | Per-pod cap on concurrent `bff:login_state:*` entries (BFF); excess `/auth/login` returns 429 |
| `gateway.logout_jti_clock_skew_seconds` | 60 | Tolerance for `iat` skew on OIDC `logout_token` -- inflates the replay-guard TTL by this amount (BFF) |
| `gateway.csrf_origins` | [] | Allowlist of acceptable `Origin` values for `/auth/*` mutations |
| `gateway.routes_configmap` | `gateway-routes` | ConfigMap with the route table |
| `gateway.signing_keys_secret` | `bff-signing-keys` | Secret with signing keys |
| `gateway.oidc.issuer_url` | (required) | Customer OIDC issuer |
| `gateway.oidc.client_id` | (required) | OIDC client ID |
| `gateway.oidc.client_secret` | (required) | OIDC client secret |

### 5.2 Observability

Metrics, logs, and audit events are described in each module's DESIGN. They share:

- One Prometheus endpoint, metric names prefixed `bff_*` or `router_*`.
- One structured-JSON log stream with `correlation_id` propagated across modules and into downstream calls.
- One audit topic in Redpanda, consumed by Audit Service.

### 5.3 Failure Handling

- Redis unreachable → 503 from `/api/*`, 401 from `/auth/*` mutations, readiness fails. No local cache, no degraded mode -- see [BFF DD-BFF-06](./bff/DESIGN.md#dd-bff-06-redis-outage--no-auth-fail-closed).
- Signing-key Secret missing → readiness fails, no requests served.
- Route ConfigMap invalid at startup → readiness fails. Invalid at runtime → keep current table, alert.
- IdP unreachable during login → 502 with retry-after. Existing sessions continue to work without contacting the IdP, since v1 does not refresh IdP tokens (see [BFF DESIGN §3.6 Session Refresh](./bff/DESIGN.md#36-interactions--sequences)).

## 6. Traceability

- **Umbrella PRD**: [PRD.md](./PRD.md)
- **Module PRDs**: [BFF PRD](./bff/PRD.md), [Router PRD](./router/PRD.md)
- **Module DESIGNs**: [BFF DESIGN](./bff/DESIGN.md), [Router DESIGN](./router/DESIGN.md)
- **Parent**: [Backend PRD](../specs/PRD.md), [Backend DESIGN](../specs/DESIGN.md)
