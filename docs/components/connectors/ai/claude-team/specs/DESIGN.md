# DESIGN — Claude Team Connector

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional Context](#4-additional-context)
  - [Source Collection Strategy](#source-collection-strategy)
  - [Cloudflare Clearance Lifecycle](#cloudflare-clearance-lifecycle)
  - [sessionKey Lifecycle and Rotation](#sessionkey-lifecycle-and-rotation)
  - [Date Cursor for code_metrics](#date-cursor-for-code_metrics)
  - [403 Graceful Handling](#403-graceful-handling)
  - [Pagination](#pagination)
  - [Operational Limitations (MVP)](#operational-limitations-mvp)
  - [Idempotence](#idempotence)
  - [Run Logging and Observability](#run-logging-and-observability)
  - [Assumptions and Risks That Affect Implementation](#assumptions-and-risks-that-affect-implementation)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

The Claude Team connector is a two-component system:

1. **`claude-team-proxy`** — a long-running Node.js service in the `insight` namespace that
   wraps a headless Chromium with `playwright-extra + puppeteer-extra-plugin-stealth`. The
   browser holds a `claude.ai` user session (via the injected `sessionKey` cookie) and is
   pre-cleared through Cloudflare's bot-management JS challenge during pod startup. The
   service exposes a minimal HTTP API (`GET /api/*`, `GET /health`) that forwards requests
   into the page's runtime via `page.evaluate(fetch(...))`, returning the upstream response
   verbatim.

2. **`claude-team`** — an Airbyte declarative source (no Python code, manifest only) that
   reads four streams from the proxy's HTTP API and writes them into Bronze under
   `bronze_claude_team.*`.

The split is **deliberate** and load-bearing. The proxy is the only component that holds
credentials (`sessionKey`) and only component that maintains stateful CF clearance. The
connector is credential-less, stateless, and identical in shape to every other declarative
connector in the repository (`claude-admin`, `claude-enterprise`, `cursor`). This isolates two
volatile concerns — auth rotation and CF anti-bot evolution — into a single rotatable Helm
release, leaving the connector's contract stable.

The connector itself is a manifest only — there is no Python code, no Docker image specific to
this connector, and no per-tenant configuration baked into the declarative bundle. Per-tenant
state lives entirely in the K8s Secret applied by the operator: `claude_org_id` plus optional
overrides (`start_date`, `proxy_url`, `insight_source_id`).

### 1.2 Architecture Drivers

#### Functional Drivers

- Deliver four Bronze streams (`claude_team_members`, `claude_team_invites`,
  `claude_team_overage_spend`, `claude_team_code_metrics`) from `claude.ai` Team-plan
  organisations into Insight.
- Support both single-day incremental code-metrics collection and operator-controlled
  multi-day backfill via the `start_date` config knob.
- Tolerate the most common failure mode (`HTTP 403 billing:view`) without failing the whole
  sync — operator can rotate to a permission-capable session later without code changes.
- Carry tenant and source stamps onto every emitted row so the universal `(tenant_id,
  source_id, unique_key)` join scope holds with sibling connectors.

#### NFR Allocation

- **Freshness** is bounded by the operator-managed schedule (Argo workflow cadence; the
  connector's own pull latency is in the seconds for snapshot streams, ~minute per day for
  `code_metrics` due to upstream's on-the-fly aggregation).
- **Credential blast radius** is bounded by the proxy: the connector holds only `claude_org_id`
  (a non-secret UUID); it never sees `sessionKey`. A compromised connector secret cannot reach
  `claude.ai`.
- **Idempotence** is bounded by `(account.uuid)` / `(uuid)` / `(account_uuid)` /
  `(metric_date, email)` primary keys on the four streams respectively. Re-running the same
  sync window converges.
- **Cloudflare resilience** is bounded by the stealth plugin's coverage. We accept the risk
  that Cloudflare may upgrade its detection in a way that defeats stealth; the architecture's
  seam (`AuthedTransport` contract in the proxy) makes swapping the browser layer cheap.

### 1.3 Architecture Layers

| Layer | What lives here | This connector's contribution |
|-------|------------------|-------------------------------|
| Source | The `claude.ai` web API + Cloudflare edge | None — external |
| Auth / CF bypass | `claude-team-proxy` headless Chromium | `src/backend/services/claude-team-proxy/` (Node.js + Playwright + Express-style HTTP) |
| Ingest | Airbyte declarative source manifest | `src/ingestion/connectors/ai/claude-team/connector.yaml` (v7.0.4), `descriptor.yaml` |
| Bronze | `bronze_claude_team.{members, invites, overage_spend, code_metrics}` | Schema declared inline in the manifest; written by the shared ClickHouse destination |
| Silver | (out of scope for v1.0; future: `staging.claude_team__*`) | — |
| Identity | (out of scope for v1.0) | — |
| Gold | (out of scope for v1.0) | — |

## 2. Principles & Constraints

### 2.1 Design Principles

- **Credential concentration.** All auth state (`sessionKey`, `__cf_bm`) lives in one pod
  managed by one Helm release. Rotation = `helm upgrade`. The connector never touches
  credentials.
- **Connector is a manifest, not a service.** No Python code, no per-connector Docker image.
  Adding or modifying streams is a YAML edit + reconcile rerun.
- **Browser as a black box behind an HTTP boundary.** The proxy exposes a stable HTTP API.
  Internally we can swap Playwright → CloakBrowser → curl-impersonate without touching the
  connector. See `src/backend/services/claude-team-proxy/src/transport/index.js` for the
  `AuthedTransport` contract.
- **Pass-through, not transform.** The proxy returns the upstream response verbatim (status,
  body, content-type). Translating, filtering, or retrying happens in the connector or its
  caller — keeping the proxy thin keeps it debuggable.
- **Honest failure modes.** A missing `billing:view` permission is a graceful skip (zero rows
  in the affected stream), not a red sync; a broken CF clearance is a hard failure on the
  proxy's `/health` endpoint, not a silent partial sync.

### 2.2 Constraints

- **One browser session per pod.** Chromium is single-tab from this connector's perspective;
  concurrency would require multiple pods (and multiple seats on `claude.ai`'s side). Not
  needed for the connector's per-day cadence.
- **Cloudflare clearance is per-pod.** A pod restart re-clears the challenge from scratch.
  K8s liveness/readiness probes are tuned with generous `initialDelaySeconds` to avoid
  flapping during the ~10–30 second clearance.
- **`claude.ai`'s web API is undocumented and unstable.** Schema changes are detected only at
  sync time via Airbyte's type-coercion errors. The connector's response schemas are derived
  from observed responses.
- **Single replica with create-before-destroy.** `strategy.maxUnavailable: 0, maxSurge: 1`
  means a rolling update spins up a new pod and waits for it to be `Ready` before draining the
  old one. There is a brief overlap window (~30s, while the new pod runs through Chromium
  boot + CF clearance) during which two browser sessions share the same `sessionKey` against
  `claude.ai`. Empirically this has not triggered session-sweep or rate-limit, but the
  alternative `maxSurge: 0 / maxUnavailable: 1` (brief downtime, no overlap) is one Helm
  upgrade away if it ever does — the connector tolerates transient 502/504 from the proxy via
  Airbyte's default retry policy.

## 3. Technical Architecture

### 3.1 Domain Model

The connector's domain has four entities — one per stream — plus the cross-cutting tenant
attribution:

```
Member        — { account.{uuid, tagged_id, full_name, email_address}, role, seat_tier,
                  created_at, updated_at }
Invite        — { uuid, email_address, role, status, created_at, expires_at }
Overage Spend — { account_uuid, account_email, account_name, seat_tier, is_enabled,
                  monthly_credit_limit, used_credits }
Code Metric   — { metric_date, email, api_key_name, status, avg_cost_per_day,
                  avg_lines_accepted_per_day, total_cost, total_lines_accepted,
                  total_sessions, last_active, prs_with_cc, total_prs,
                  prs_with_cc_percentage }
```

All four extended with the standard attribution columns:

```
tenant_id           — Insight tenant UUID, from config
source_id           — Source instance id, from config (resolved by reconcile from the
                       insight.cyberfabric.com/source-id Secret annotation)
unique_key          — Per-row composite: '{tenant_id}-{source_id}-{<natural-key>}', where
                       <natural-key> is account.uuid for members, uuid for invites,
                       account_uuid for overage_spend, and date+email for code_metrics
collected_at        — ISO timestamp of sync run
data_source         — constant 'insight_claude_team'
```

### 3.2 Component Model

The proxy is layered with explicit contracts. Reading top-down:

```
┌──────────────────────────────────────────────────────────────┐
│ Layer 1: HTTP API (node:http)                                │
│  src/server.js                                               │
│  Routes:                                                     │
│   GET /api/*  → transport.fetch(upstreamBaseUrl + path)      │
│   GET /health → { ready: transport.isReady() }               │
└────────────────────▼─────────────────────────────────────────┘
                     │
┌────────────────────▼─────────────────────────────────────────┐
│ Layer 2: AuthedTransport contract (JSDoc, no runtime cost)   │
│  src/transport/index.js                                      │
│                                                              │
│  interface AuthedTransport {                                 │
│    init(): Promise<void>     // CF challenge + session boot  │
│    fetch(url, opts?):                                        │
│      Promise<{status, body, headers}>                        │
│    isReady(): boolean                                        │
│    close(): Promise<void>                                    │
│    kind: 'playwright' | 'mock' | …                           │
│    upstreamBaseUrl: string                                   │
│  }                                                           │
└────────────────────▲─────────────────────────────────────────┘
                     │
┌────────────────────┴─────────────────────────────────────────┐
│ Layer 3: PlaywrightTransport (one of N possible impls)       │
│  src/transport/playwright.js                                 │
│                                                              │
│  • chromium.launch + stealth plugin                          │
│  • addCookies(sessionKey, domain: claude.ai)                 │
│  • page.goto(claude.ai) → wait for CF clearance              │
│  • fetch(url) implemented as page.evaluate(fetch)            │
└──────────────────────────────────────────────────────────────┘
```

The seam at Layer 2 is the **architectural escape hatch**. If Cloudflare's bot management
upgrades in a way that defeats Playwright + stealth, we ship a `CloakBrowserTransport` or
`CurlImpersonateTransport` and update one factory call — Layer 1 is unaware.

The connector (in `src/ingestion/connectors/ai/claude-team/`) is a YAML manifest. There is no
runtime component to model — Airbyte's declarative interpreter executes it.

### 3.3 API Contracts

**Proxy ↔ Connector** (in-cluster, plain HTTP):

```
GET http://claude-team-proxy.insight.svc.cluster.local:3000/api/{path}
    → Proxy executes: page.evaluate(fetch(`https://claude.ai/api/{path}`))
    → Returns: upstream status (verbatim)
              upstream body (verbatim, content-type preserved)
              + x-proxy-transport: playwright (debug header)

GET http://…:3000/health
    → 200 {"ready": true, "transport": "playwright"} when transport.isReady()
    → 503 {"ready": false} during init or after transport failure
```

**Proxy ↔ `claude.ai`** (in-page fetch via Chromium):

| Path | Status | Records | Notes |
|------|--------|---------|-------|
| `GET /api/organizations/{org_id}/members` | 200 | JSON array of members | Single response, no pagination |
| `GET /api/organizations/{org_id}/invites` | 200 | JSON array of invites | Single response, only pending |
| `GET /api/organizations/{org_id}/overage_spend_limits?page=N&per_page=100` | 200 / 403 | `{items, page, per_page, total, total_pages}` | 403 when session lacks `billing:view` |
| `GET /api/claude_code/metrics_aggs/users?start_date=D&end_date=D&limit=100&offset=N&organization_uuid=…&customer_type=claude_ai&subscription_type=team` | 200 | `{users: [...], pagination, …}` | Per-day, offset-paginated within day |

### 3.4 Internal Dependencies

- **`src/ingestion/reconcile-connectors/`** — reads `descriptor.yaml` + Secret, publishes
  manifest to Airbyte, creates Source / Connection / CronWorkflow. Connector contributes
  only `connector.yaml` + `descriptor.yaml`.
- **Shared ClickHouse destination** — same destination as every other Insight connector.
  Reconcile reuses the existing `2b287319-be21-4e1b-ad27-adf08293b042` destination id.

### 3.5 External Dependencies

- **Cloudflare edge** in front of `claude.ai`. Serves the JS challenge on first contact.
- **`claude.ai` web app** — undocumented internal API. Schema may change without notice.
- **GitHub Container Registry (`ghcr.io`)** — destination for the proxy's image when CI is
  wired (not in v1.0 scope).
- **Playwright Chromium binary** — downloaded at proxy image-build time via
  `npx playwright install --with-deps chromium`.

### 3.6 Interactions & Sequences

**Boot sequence (proxy pod startup):**

```
k8s Scheduler          Proxy pod (Chromium + Node)            claude.ai (Cloudflare + app)
     │                          │                                       │
     │── create pod ──────────► │                                       │
     │                          │── chromium.launch(headless) ──┐       │
     │                          │                               │       │
     │                          │ ◄──── browser ready ──────────┘       │
     │                          │                                       │
     │                          │── context.addCookies(sessionKey)      │
     │                          │── page.goto(https://claude.ai) ────► │ (CF intercepts)
     │                          │                                       │
     │                          │                                  ◄─── HTTP 403 + JS challenge
     │                          │   Chromium executes JS challenge      │
     │                          │── retry after CF JS handler ───────► │
     │                          │                                  ◄─── HTTP 200 + Set-Cookie: __cf_bm
     │                          │                                       │
     │                          │── waitForFunction(!title=='Just a moment')
     │                          │── transport.ready = true              │
     │                          │── HTTP server starts on :3000         │
     │ ◄── readiness probe ─── │                                       │
     │     GET /health → 200    │                                       │
     │                          │                                       │
     │── route Service ────────► │                                       │
```

**Sync sequence (for a single stream, e.g. `claude_team_members`):**

```
Airbyte worker          Proxy pod                       claude.ai
     │                       │                               │
     │── GET /api/orgs/X/members ───►│                       │
     │                       │                               │
     │                       │── page.evaluate(fetch(...)) ─►│
     │                       │   (in-page; carries sessionKey + __cf_bm)
     │                       │                          ◄────│ 200 + JSON array
     │                       │                               │
     │                  ◄─── proxy passes status+body verbatim
     │                       │                               │
     │── parse records ─────►│                               │
     │── stamp tenant_id, … ─►│                               │
     │── ship to ClickHouse  │                               │
```

**sessionKey expiry sequence:**

```
Sync run N            Sync run N+1 (after cookie expires)      Operator
     │                       │                                     │
     │── GET /api/orgs ────► │                                     │
     │                       │── page.evaluate(fetch(...))         │
     │                       │   claude.ai returns 401             │
     │                  ◄─── │ proxy passes 401 verbatim           │
     │                       │                                     │
Sync fails; Airbyte reports red job
     │                       │                              ──────►│ Alert / sees red job
     │                       │                                     │
     │                       │                                     ├── extracts new sessionKey from DevTools
     │                       │                                     ├── helm upgrade --reuse-values --set-string sessionKey=…
     │                       │ ◄── new pod with new cookie ────────┘
     │                       │ ◄── browser re-clears CF ───────────
     │                       │                                     │
Next scheduled sync (N+2) succeeds against the new cookie.
```

### 3.7 Database schemas & tables

Four Bronze tables under `bronze_claude_team`. All include the standard `_airbyte_*` columns
emitted by the ClickHouse destination (`_airbyte_raw_id`, `_airbyte_extracted_at`,
`_airbyte_meta`, `_airbyte_generation_id`).

**`bronze_claude_team.claude_team_members`** — primary key `account.uuid`

| Column | Type | Source |
|--------|------|--------|
| `tenant_id` | `String` | injected from config |
| `source_id` | `String` | injected from config |
| `unique_key` | `String` | composite `{tenant_id}-{source_id}-{account.uuid}` |
| `collected_at` | `String` (ISO) | injected at sync time |
| `data_source` | `String` ('insight_claude_team') | injected |
| `account` | `Nullable(JSON)` | upstream — nested object with uuid, tagged_id, full_name, email_address |
| `role` | `Nullable(String)` | upstream |
| `seat_tier` | `Nullable(String)` | upstream |
| `created_at` | `Nullable(String)` | upstream — ISO |
| `updated_at` | `Nullable(String)` | upstream — ISO |

**`bronze_claude_team.claude_team_invites`** — primary key `uuid`

| Column | Type | Source |
|--------|------|--------|
| `tenant_id` / `source_id` / `collected_at` / `data_source` | as above | injected |
| `unique_key` | `String` | composite `{tenant_id}-{source_id}-{uuid}` |
| `uuid` | `String` | upstream |
| `email_address` | `Nullable(String)` | upstream |
| `role` | `Nullable(String)` | upstream |
| `status` | `Nullable(String)` | upstream |
| `created_at` | `Nullable(String)` | upstream |
| `expires_at` | `Nullable(String)` | upstream |

**`bronze_claude_team.claude_team_overage_spend`** — primary key `account_uuid`

| Column | Type | Source |
|--------|------|--------|
| `tenant_id` / `source_id` / `collected_at` / `data_source` | as above | injected |
| `unique_key` | `String` | composite `{tenant_id}-{source_id}-{account_uuid}` |
| `account_uuid` | `String` | upstream |
| `account_email` | `Nullable(String)` | upstream |
| `account_name` | `Nullable(String)` | upstream |
| `seat_tier` | `Nullable(String)` | upstream |
| `is_enabled` | `Nullable(Bool)` | upstream |
| `monthly_credit_limit` | `Nullable(Float64)` | upstream |
| `used_credits` | `Nullable(Float64)` | upstream |

Empty unless the `sessionKey` user has `billing:view`.

**`bronze_claude_team.claude_team_code_metrics`** — primary key `(metric_date, email)`

| Column | Type | Source |
|--------|------|--------|
| `tenant_id` / `source_id` / `collected_at` / `data_source` | as above | injected |
| `unique_key` | `String` | composite `{tenant_id}-{source_id}-{metric_date}-{email}` |
| `metric_date` | `String` (YYYY-MM-DD) | injected from cursor's `start_time` |
| `email` | `String` | upstream |
| `api_key_name` | `Nullable(String)` | upstream |
| `status` | `Nullable(String)` | upstream |
| `avg_cost_per_day` | `Nullable(String)` | upstream — decimal-as-string per `claude.ai`'s convention |
| `avg_lines_accepted_per_day` | `Nullable(Float64)` | upstream |
| `total_cost` | `Nullable(String)` | upstream — decimal-as-string |
| `total_lines_accepted` | `Nullable(Int64)` | upstream |
| `total_sessions` | `Nullable(Int64)` | upstream |
| `last_active` | `Nullable(String)` | upstream — ISO |
| `prs_with_cc` | `Nullable(Int64)` | upstream |
| `total_prs` | `Nullable(Int64)` | upstream |
| `prs_with_cc_percentage` | `Nullable(Float64)` | upstream |

### 3.8 Deployment Topology

```
┌──────────────────────────────────────────────────────────────────────┐
│ kind / production cluster                                            │
│                                                                      │
│ Namespace: insight                                                   │
│ ┌──────────────────────────────────────────────────────┐             │
│ │ Helm release: claude-team-proxy                      │             │
│ │   • Deployment (1 replica, maxUnavailable=0)         │             │
│ │   • Service ClusterIP :3000                          │             │
│ │   • Secret (managed by Helm — holds sessionKey)      │             │
│ └───────────────────────▲──────────────────────────────┘             │
│                         │ HTTP (in-cluster)                          │
│                         │                                            │
│ ┌───────────────────────┴──────────────────────────────┐             │
│ │ Secret: insight-claude-team-main (operator-applied)  │             │
│ │   • stringData.claude_org_id                         │             │
│ │   • annotations: connector=claude-team,              │             │
│ │                  source-id=claude-team-main          │             │
│ └──────────────────────────────────────────────────────┘             │
│                                                                      │
│ Namespace: airbyte                                                   │
│ ┌──────────────────────────────────────────────────────┐             │
│ │ Airbyte server / worker / temporal / db / minio      │             │
│ │ Holds:                                               │             │
│ │   • Source definition (declarative manifest)         │             │
│ │   • Source instance (configured per claude_org_id)   │             │
│ │   • Connection (Source → ClickHouse destination)     │             │
│ └──────────────────────────────────────────────────────┘             │
│                                                                      │
│ Namespace: argo                                                      │
│ ┌──────────────────────────────────────────────────────┐             │
│ │ CronWorkflow: claude-team-…                          │             │
│ │   schedule from descriptor.yaml (`0 4 * * *`)        │             │
│ └──────────────────────────────────────────────────────┘             │
│                                                                      │
│ Namespace: data                                                      │
│ ┌──────────────────────────────────────────────────────┐             │
│ │ ClickHouse                                           │             │
│ │   bronze_claude_team.{members, invites,              │             │
│ │     overage_spend, code_metrics}                     │             │
│ └──────────────────────────────────────────────────────┘             │
└──────────────────────────────────────────────────────────────────────┘
```

## 4. Additional Context

### Source Collection Strategy

The four streams divide into two categories by collection style:

- **Snapshot streams** (`members`, `invites`, `overage_spend`) — each sync re-reads the entire
  current state from `claude.ai`. Configured as `full_refresh + overwrite` in Airbyte. Previous
  rows are truncated before the new batch lands. Implication: invites accepted between syncs
  disappear from Bronze (we cannot reconstruct historical invite-lifecycle events from this
  endpoint). Members removed from the org disappear from Bronze (Silver layer is responsible
  for soft-delete tracking if needed).

- **Cursor stream** (`code_metrics`) — uses `DatetimeBasedCursor` walking one day per request.
  Default backfill anchor is 7 days back; operator can override via `start_date` config.
  Hard floor `2025-11-24` matches the earliest data point observed in the data_collector audit
  for the reference organisation. Records are written with composite PK `(metric_date, email)`
  so re-running the same window converges deterministically.

### Cloudflare Clearance Lifecycle

The CF challenge is a one-time-per-browser event. After `page.goto(https://claude.ai)` and the
`waitForFunction(() => !document.title.includes('Just a moment'))` resolves, the browser carries
a `__cf_bm` cookie that Cloudflare's edge accepts on all subsequent requests to the same hostname.

The `__cf_bm` cookie has a ~30-minute lifetime. On expiry, Cloudflare returns a new
`Set-Cookie: __cf_bm` header on the next request — Chromium picks it up automatically. The
connector never sees this cycle; the proxy's `page.evaluate(fetch)` benefits from the
browser's cookie store.

Stealth plugin coverage (`puppeteer-extra-plugin-stealth` ~v2.11) is sufficient for the JS
challenge tier as of v1.0. If Cloudflare upgrades to CAPTCHA, this approach breaks; mitigation
is the architectural seam (`AuthedTransport` swap to CloakBrowser or another solver).

### sessionKey Lifecycle and Rotation

The `sessionKey` cookie is issued by `claude.ai` on user login. Lifetime is not documented but
empirically lasts at least one week of inactive use; renewed on web-UI activity.

Invalidation triggers:
- User-initiated logout (rare for a maintained service account)
- Password change
- Anthropic's server-side session sweep (heuristic; not visible to operator)

Rotation flow:
1. Operator opens `claude.ai` in browser → DevTools → Application → Cookies → copies `sessionKey`.
2. `helm upgrade claude-team-proxy ./helm --namespace insight --reuse-values --set-string sessionKey="$NEW_VAL"`.
3. Helm rolls a new pod. Chromium boots with the new cookie, re-clears CF.
4. K8s drops the old pod after the new one is ready.

The connector and its Secret are untouched. Sync state is preserved (cursors live in
Airbyte's database, not in the proxy).

### Date Cursor for code_metrics

The cursor declaration in `connector.yaml`:

```yaml
incremental_sync:
  type: DatetimeBasedCursor
  cursor_field: metric_date
  cursor_datetime_formats: ["%Y-%m-%d"]
  datetime_format: "%Y-%m-%d"
  start_datetime:
    datetime: "{{ config.get('start_date') or day_delta(-7, format='%Y-%m-%d') }}"
    min_datetime: "2025-11-24"
  end_datetime:
    datetime: "{{ day_delta(-1, format='%Y-%m-%d') }}"
  step: P1D
  cursor_granularity: P1D
  start_time_option:
    field_name: start_date
  end_time_option:
    field_name: end_date
```

The API requires both `start_date` and `end_date` as **inclusive** single-day filters. Both
inject the cursor's start value (the day being walked) — `start_date=2026-05-20&end_date=2026-05-20`
returns one day's data. Step `P1D` increments by one calendar day per slice.

`metric_date` is **not** a field returned by the API in per-user objects (the API returns it
only as top-level `start_date` once per response). The connector injects it into every record
via an `AddFields` transformation using `{{ stream_interval.start_time }}`. Without this,
records from different days would collide on `(metric_date, email)` because `metric_date`
would be empty for all.

### 403 Graceful Handling

The `claude_team_overage_spend` stream's requester carries an `error_handler`:

```yaml
error_handler:
  type: DefaultErrorHandler
  response_filters:
    - http_codes: [403]
      action: IGNORE
      error_message: "claude.ai returned 403 — sessionKey lacks billing:view permission. Stream skipped, sync continues."
```

When the upstream returns `HTTP 403 permission_error` (the proxy passes it through verbatim),
Airbyte's CDK matches the filter and marks the response as "no records, no failure" instead of
raising. The stream commits zero records; the overall sync continues to the next stream and
reports `succeeded`.

This is intentionally a **stream-level** policy (not connector-level) so that any future stream
with a different failure model (e.g. a 401 that should fail loud) is unaffected.

### Pagination

Two pagination styles, one per applicable stream:

- **`PageIncrement`** for `claude_team_overage_spend` — request carries `?page=N&per_page=100`,
  starting at 1. Stop condition: response carries fewer than `per_page` items.
- **`OffsetIncrement`** for `claude_team_code_metrics` — request carries `?offset=N&limit=100`,
  starting at 0. Stop condition: response carries fewer than `limit` items, or the API's own
  `pagination.has_next: false` (the paginator stops naturally on the former; the latter is
  documentation).

`claude_team_members` and `claude_team_invites` return the full set in one response; no
pagination configured.

### Operational Limitations (MVP)

Two known operational corners that v1.0 trades against simplicity. Both are flagged
during PR #536 review by @dzarlax; both are deliberate scope decisions, not bugs.

**1. `/health` reports init-once, not live-operational.** `transport.isReady()`
flips to `true` once at the end of `init()` and is never re-validated. If, between
init and the next sync:

- the `sessionKey` cookie expires (claude.ai issues 401 on `/api/*`),
- Cloudflare changes its posture and starts re-issuing challenges, OR
- the Chromium process partially dies (zygote alive but a child segfaulted),

…the readiness probe still passes and the pod stays in Service rotation. The
operator finds out only when a sync fails. Mitigation paths if this becomes
painful in production:

- Active probe — `/health` makes a cheap upstream call (e.g. `GET /api/organizations`)
  every N seconds and caches the result. Trades upstream load for liveness fidelity.
- Sidecar — separate prober pod that pings `/api/*` periodically and bumps an
  `is_operational` flag in the proxy's state. Decouples probing cost from the
  hot path.

Out of scope for v1.0 because sync cadence is hourly-to-daily and a stale-ready
pod fails at next sync, which the operator already monitors via Airbyte.

**2. Cloudflare-clearance detection is title-string-based.** The proxy waits for
`!document.title.includes('Just a moment')` — fragile. Cloudflare can change the
interstitial wording, the page structure, or move to a CAPTCHA tier at any time
without notice; the `waitForFunction` would either return immediately (false
clearance, subsequent requests 403) or hang to the startup-timeout (pod won't
become ready). Mitigation when this happens:

- Lightweight authenticated upstream probe — after the title check, issue a
  `GET /api/organizations` from the page context; require HTTP 200 + JSON body
  before flipping `ready=true`. Robust to interstitial-wording changes but
  costs one upstream call per pod start.
- Browser-engine swap — replace Playwright + stealth with CloakBrowser CDP,
  which patches the bot detection at the Chromium source level rather than via
  JS injection. The `AuthedTransport` seam (§3.2) makes this a one-file change.

Out of scope for v1.0 because no Cloudflare changes observed to date and the
detection has been stable for the kind / staging soak window. Flagged here so
the operator does not assume the detection is more robust than it is.

### Idempotence

- `full_refresh + overwrite` truncates the destination table before each load, so
  `members` / `invites` / `overage_spend` are idempotent by construction.
- `code_metrics` PK `(metric_date, email)` plus Airbyte's destination-side merge make re-runs
  of the same date window safe. Re-running with an earlier `start_date` does not duplicate the
  newer dates — the destination merges on PK.

### Run Logging and Observability

The proxy emits JSON-line logs (one record per line) on stdout:

```json
{"ts":"2026-05-25T09:30:00.123Z","level":"info","msg":"transport.init starting","headless":true,"upstreamBaseUrl":"https://claude.ai"}
{"ts":"2026-05-25T09:30:12.456Z","level":"info","msg":"transport ready","ms":12333,"page_title":"Claude"}
{"ts":"2026-05-25T09:30:15.000Z","level":"info","msg":"proxy","upstream":"https://claude.ai/api/…","status":200,"size":44963,"ms":818}
```

Fields:
- `ts` — UTC ISO 8601.
- `level` — `info` / `warn` / `error`.
- `msg` — short description; structured data lives in additional fields.
- Per-request: `upstream`, `status`, `size`, `ms`.

k8s collects stdout; downstream parser (fluentbit, loki) reads JSON line-by-line.

Airbyte's own sync logs are visible in the Airbyte UI or via API (`/api/v1/jobs/get`).

### Assumptions and Risks That Affect Implementation

- **The `claude.ai` web API is internally consistent across pagination pages.** We assume that
  page N+1 of `/overage_spend_limits` returns the same fields as page N. Schema drift mid-pagination
  would cause partial records.
- **Cloudflare's `__cf_bm` is hostname-scoped.** Subdomain hops (e.g. `assets.claude.ai`) would
  require additional clearance flows. The endpoints we use are all on `claude.ai` directly, so
  this is moot in v1.0.
- **The proxy's single browser context is enough.** Browsers can technically open multiple
  pages per context; if future scale demands parallelism, the proxy can be extended without
  contract changes. The connector's per-stream pace is much slower than browser concurrency
  limits.
- **`sessionKey` rotation is operator-driven.** We accept the operational burden of manual
  rotation for v1.0 per #458 decision. Future: automated email-code Playwright flow with
  IMAP polling.

## 5. Traceability

| Spec section | Implementation |
|--------------|----------------|
| §3.2 Component Model — Layer 1 HTTP API | `src/backend/services/claude-team-proxy/src/server.js` |
| §3.2 Component Model — Layer 2 contract | `src/backend/services/claude-team-proxy/src/transport/index.js` |
| §3.2 Component Model — Layer 3 Playwright | `src/backend/services/claude-team-proxy/src/transport/playwright.js` |
| §3.3 API Contracts — proxy ↔ connector | `src/backend/services/claude-team-proxy/src/server.js` routing |
| §3.6 Boot sequence | `src/backend/services/claude-team-proxy/src/index.js` |
| §3.7 Database schemas | `src/ingestion/connectors/ai/claude-team/connector.yaml` (`streams[*].schema_loader`) |
| §3.8 Deployment topology | `src/backend/services/claude-team-proxy/helm/`, `src/ingestion/secrets/connectors/claude-team.yaml.example` |
| §4 Cloudflare clearance lifecycle | `src/backend/services/claude-team-proxy/src/transport/playwright.js` `init()` method |
| §4 Date cursor for code_metrics | `src/ingestion/connectors/ai/claude-team/connector.yaml` `streams[3].incremental_sync` |
| §4 403 Graceful Handling | `src/ingestion/connectors/ai/claude-team/connector.yaml` `streams[2].retriever.requester.error_handler` |
| §4 Pagination | `src/ingestion/connectors/ai/claude-team/connector.yaml` `streams[2,3].retriever.paginator` |
