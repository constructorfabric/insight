# PRD — Zulip-Proxy Connector

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
  - [5.1 User Directory Collection](#51-user-directory-collection)
  - [5.2 Aggregated Message Activity](#52-aggregated-message-activity)
  - [5.3 Connector Operations and Data Integrity](#53-connector-operations-and-data-integrity)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-001 Refresh Zulip User Directory](#uc-001-refresh-zulip-user-directory)
  - [UC-002 Collect Aggregated Message Activity](#uc-002-collect-aggregated-message-activity)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Zulip-Proxy Connector extracts collaboration signals from Zulip into the Insight platform's
Bronze layer through a self-hosted aggregation proxy rather than the public Zulip REST API. It
covers the Zulip user directory as the identity anchor and aggregated message counts per sender
over time as the primary asynchronous-communication signal. Message bodies are never collected:
the proxy pre-aggregates counts so that the connector never sees individual message text.

The connector is a strict siblings of the existing direct-API Zulip connector specification at
[../zulip](../). Both deliver the same Bronze tables (`zulip_users`, `zulip_messages`) and feed
the same Silver targets, but they reach the source by different paths: the direct connector uses
HTTP Basic Auth against `https://{realm}.zulipchat.com/api/v1/`; this connector uses Bearer-token
auth against an operator-controlled proxy that aggregates counts and serves them on a private
endpoint.

### 1.2 Background / Problem Statement

Some Zulip deployments cannot be reached directly from the Insight ingestion cluster:

- Self-hosted Zulip realms behind a corporate network or VPN cannot expose the Zulip REST API to
  Insight without changing perimeter rules.
- Compliance constraints in some tenants forbid Insight from holding any credential that can read
  message bodies, even if the connector never persists them.
- Tenants that operate multiple Zulip realms want a single per-tenant endpoint that pre-aggregates
  counts before any cross-realm shipping happens.

The proxy solves these problems by living inside the tenant's network, holding the Zulip
credentials, performing the per-sender aggregation, and exposing two read-only endpoints
(`/api/users`, `/api/messages`) protected by a Bearer token issued to the Insight connector.

**Target Users**:

- Platform operators who deploy and rotate the proxy's Bearer token and configure Insight to read
  from it.
- Data analysts who consume Zulip message activity and user directory data in Silver and Gold
  layers.
- Compliance and security stakeholders who require that Insight never holds Zulip's primary
  bot/API credentials.

**Key Problems Solved**:

- Bringing private/air-gapped Zulip realms into Insight without opening direct API access.
- Allowing tenants to restrict Insight's credential scope to "read aggregated counts" rather than
  "read message bodies".
- Providing a uniform Bronze schema regardless of whether the upstream Zulip is reached directly
  or through a proxy.

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- Insight receives a refreshed Zulip user directory at least once per scheduled run for every
  configured proxy instance (Baseline: not collected from proxied Zulip realms; Target: v1.0).
- Insight receives aggregated Zulip message counts per sender per period (`created_at`) within the
  configured collection cadence (Baseline: not collected from proxied Zulip realms; Target: v1.0).
- The connector never holds Zulip primary credentials and never persists message body content
  (Baseline: scope undefined for proxied realms; Target: v1.0).

**Capabilities**:

- Pull the Zulip user directory through the proxy and persist it as `zulip_users` in Bronze.
- Pull aggregated message-count records through the proxy and persist them as `zulip_messages`,
  incrementally on `created_at`.
- Stamp every Bronze row with `tenant_id`, `source_id`, and `unique_key` per repo conventions so
  Silver can join across sources without ambiguity.
- Allow per-tenant overrides for proxy host, Bearer token, backfill start date, and server-side
  throttle hint without modifying the connector image.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Zulip | Open-source team chat platform; primary source of the collaboration signal this connector ingests. |
| Zulip Proxy | Self-hosted aggregation service operated inside the tenant network. Holds Zulip credentials, performs per-sender count aggregation, exposes `/api/users` and `/api/messages` over Bearer auth. |
| Bearer Token | Opaque secret issued by the proxy operator to the Insight connector; rotated independently of Zulip credentials. |
| Aggregated Message Record | One row of `zulip_messages` representing the message count for a single sender within one aggregation bucket (period defined by the proxy; carried in `created_at`). |
| Backfill Start Date | Earliest `created_at` the connector requests on its very first sync. Bounds the historical window. |
| Throttle Hint | Server-side pacing parameter forwarded to the proxy on `/api/messages` requests; tells the proxy how aggressively it may aggregate-and-stream. |

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-insightspec-actor-zulip-proxy-operator`

**Role**: Deploys the Zulip-proxy service inside the tenant network, mints the Bearer token,
configures the Insight K8s Secret with the proxy URL and token, and monitors run health.
**Needs**: A connector that reads only from the proxy endpoints (no direct Zulip credentials),
respects the proxy's pacing hints, and surfaces failures so the operator can detect when the
proxy is down or the token has expired.

#### Data Analyst

**ID**: `cpt-insightspec-actor-zulip-proxy-analyst`

**Role**: Uses Zulip message activity and user directory data in Silver and Gold for collaboration
analytics, cross-platform comparisons against M365/Zoom/Slack, and team-level reporting.
**Needs**: Stable per-sender message counts indexed by `created_at`; a Zulip user directory with
emails for identity resolution.

#### Compliance Stakeholder

**ID**: `cpt-insightspec-actor-zulip-proxy-compliance`

**Role**: Defines the credential blast radius and content-collection policy for the tenant.
**Needs**: Confidence that Insight cannot, under any operational path, observe Zulip message
bodies — only aggregated counts produced by the tenant-controlled proxy.

### 2.2 System Actors

#### Zulip Proxy

**ID**: `cpt-insightspec-actor-zulip-proxy-source`

**Role**: External service operated inside the tenant network. Exposes `/api/users` and
`/api/messages` over HTTPS-or-HTTP with Bearer-token authentication. Performs aggregation of
Zulip message counts per sender and per period before responding.

#### Bronze Ingestion Platform

**ID**: `cpt-insightspec-actor-zulip-proxy-bronze-ingestion`

**Role**: Receives connector output, persists `bronze_zulip_proxy.users` and
`bronze_zulip_proxy.messages`, and makes them available to the Silver layer for identity
resolution and collaboration analytics.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- The proxy is reachable from the ingestion cluster via the URL provided in the K8s Secret. No
  egress route to public Zulip is required.
- The proxy enforces Bearer authentication. The token is opaque to the connector and rotated by
  the operator out-of-band; rotation invalidates outstanding tokens immediately and the connector
  must surface 401 errors as a credential-rotation signal.
- The proxy is the only place where Zulip primary credentials live. The connector MUST NOT be
  configured with Zulip bot email or API key.
- The proxy may impose its own pacing through `throttle` query parameters and HTTP 429 responses
  with `Retry-After`; the connector MUST respect both.
- The connector operates as a scheduled batch collector, not a real-time event stream. The
  scheduling is owned by the surrounding ingestion platform (Argo + descriptor `schedule`).

## 4. Scope

### 4.1 In Scope

- Collection of the Zulip user directory through `/api/users` and persistence as
  `bronze_zulip_proxy.users`.
- Incremental collection of aggregated per-sender message counts through `/api/messages` and
  persistence as `bronze_zulip_proxy.messages`, with `created_at` as the incremental cursor.
- Tenant- and source-stamping (`tenant_id`, `source_id`, `unique_key`) on every Bronze row per
  the ingestion-data-flow ADRs.
- Identity-resolution support: Bronze users feed the Identity Manager via the
  `zulip_proxy__identity_inputs` Silver model (email → canonical `person_id`).
- A best-effort historical backfill window controlled by the `zulip_proxy_start_date` Secret field
  on first sync.

### 4.2 Out of Scope

- Direct collection from the Zulip REST API (covered by the sibling `zulip` connector spec).
- Collection of Zulip message bodies, attachments, reactions, or thread structure (the proxy does
  not expose them and Insight has no use case for them).
- Channel/stream metadata, subscriptions, or per-channel message decomposition; only sender-level
  aggregates are returned by the proxy.
- Per-message timestamps below the proxy's aggregation bucket granularity.
- Proxy deployment, configuration, or operational monitoring; that is the operator's
  responsibility outside the connector.
- Cross-realm reconciliation when a single proxy aggregates multiple Zulip realms; the proxy is
  expected to flatten realm context into the records it emits.

## 5. Functional Requirements

### 5.1 User Directory Collection

#### Collect Zulip User Directory

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-user-directory`

The connector **MUST** request the full Zulip user directory from the proxy on every scheduled run
and persist each user record into `bronze_zulip_proxy.users` with stable source-native fields
(`id`, `uuid`, `email`, `full_name`, `role`, `is_active`, `recipient_id`).

**Rationale**: Without a refreshed directory, downstream identity resolution drifts as users join
or leave the realm.

**Actors**: `cpt-insightspec-actor-zulip-proxy-source`, `cpt-insightspec-actor-zulip-proxy-analyst`

#### Stamp Tenant and Source on User Records

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-user-stamping`

Every emitted user row **MUST** carry `tenant_id`, `source_id`, and a `unique_key` computed as
`{tenant}-{source}-{zulip_user_id}`.

**Rationale**: Insight's Silver / Identity layer uses these stamps as the universal join scope;
omitting them silently dissolves the row into the cross-tenant pool.

**Actors**: `cpt-insightspec-actor-zulip-proxy-bronze-ingestion`

### 5.2 Aggregated Message Activity

#### Collect Aggregated Message Counts Incrementally

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-message-incremental`

The connector **MUST** request aggregated message records from the proxy through `/api/messages`
incrementally, using the largest `created_at` seen in the previous sync as the lower bound of the
next sync's window.

**Rationale**: Re-pulling the full history on every sync wastes proxy CPU (the aggregation step
is non-trivial) and is unnecessary because the proxy already partitions records by `created_at`.

**Actors**: `cpt-insightspec-actor-zulip-proxy-source`, `cpt-insightspec-actor-zulip-proxy-operator`

#### Honor Backfill Start Date

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-backfill-start-date`

On first sync (no prior state) the connector **MUST** request data starting from the date
configured in `zulip_proxy_start_date` and **MUST NOT** request data older than that anchor.

**Rationale**: Older history may not be retained by the proxy; the operator sets a known-safe
floor.

**Actors**: `cpt-insightspec-actor-zulip-proxy-operator`

#### Forward Server-Side Throttle Hint

- [ ] `p2` - **ID**: `cpt-insightspec-fr-zulip-proxy-throttle-hint`

The connector **MUST** forward the configured `zulip_proxy_throttle_ms` value as the `throttle`
query parameter on `/api/messages` requests when set.

**Rationale**: The proxy uses this hint to pace its own aggregation work; ignoring it can cause
the proxy to either over-shed under load or under-respond.

**Actors**: `cpt-insightspec-actor-zulip-proxy-source`, `cpt-insightspec-actor-zulip-proxy-operator`

#### Stamp Tenant and Source on Message Records

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-message-stamping`

Every emitted message-aggregate row **MUST** carry `tenant_id`, `source_id`, and a `unique_key`
computed as `{tenant}-{source}-{record.uniq}`.

**Rationale**: Same as 5.1.2 — Silver depends on the stamps.

**Actors**: `cpt-insightspec-actor-zulip-proxy-bronze-ingestion`

#### Exclude Message Content

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-no-content`

The connector **MUST NOT** request, parse, or persist any Zulip message body, attachment, or
reaction content. The proxy is responsible for never exposing such content; the connector
**MUST** treat any content field the proxy may accidentally include as out-of-scope and drop it.

**Rationale**: Content collection is explicitly out of scope and is the primary compliance
boundary that motivates the proxy in the first place.

**Actors**: `cpt-insightspec-actor-zulip-proxy-compliance`

### 5.3 Connector Operations and Data Integrity

#### Idempotent Overlap Handling

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-idempotence`

The connector **MUST** be idempotent across overlapping collection windows. Repeated or recovery
runs **MUST NOT** create duplicate `zulip_users` or `zulip_messages` rows in Silver.

**Rationale**: The Bronze→RMT promotion (`promote_bronze_to_rmt`) deduplicates by `unique_key`
on merge, so the connector's contribution is to keep `unique_key` deterministic across reruns.

**Actors**: `cpt-insightspec-actor-zulip-proxy-operator`, `cpt-insightspec-actor-zulip-proxy-analyst`

#### Surface Credential Failures

- [ ] `p1` - **ID**: `cpt-insightspec-fr-zulip-proxy-401-surfaces`

A 401 response from the proxy on any stream **MUST** fail the run with a clear log message that
identifies the connector and source-id, so the operator can rotate the Bearer token.

**Rationale**: Silent retries on a rotated token waste sync cycles and delay operator
intervention.

**Actors**: `cpt-insightspec-actor-zulip-proxy-operator`

#### Tolerate Transient Failures

- [ ] `p2` - **ID**: `cpt-insightspec-fr-zulip-proxy-transient-resilience`

The connector **MUST** retry with exponential backoff on transient HTTP failures (5xx, 429, 503)
up to a documented bound and **MUST** honor `Retry-After` when present.

**Rationale**: Proxy availability is operator-managed; transient blips are expected and must not
block the run.

**Actors**: `cpt-insightspec-actor-zulip-proxy-operator`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Freshness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-zulip-proxy-freshness`

The connector **MUST** make newly available aggregated message activity available to downstream
consumers within 24 hours of the proxy receiving it from upstream Zulip, under normal operating
conditions.

#### Credential Blast Radius

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-zulip-proxy-credential-blast-radius`

The credentials held by the Insight ingestion cluster on behalf of this connector **MUST** be
limited to the proxy Bearer token. Compromise of this token **MUST NOT** grant access to Zulip
message bodies or to administrative Zulip endpoints.

#### Idempotent State

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-zulip-proxy-state-idempotence`

After any successful or failed sync, the cursor state **MUST** be either advanced to the largest
observed `created_at` or unchanged from the previous run. Partial advancement on failure is
forbidden.

### 6.2 NFR Exclusions

- **End-to-end message latency**: out of scope; the connector is batch-scheduled.
- **Content-scanning performance**: out of scope; the connector does not see content.
- **Realm-level access control reporting**: out of scope; the proxy normalizes access.

## 7. Public Library Interfaces

### 7.1 Public API Surface

None — the connector is a manifest-driven Airbyte declarative source.

### 7.2 External Integration Contracts

#### Zulip-Proxy Source Contract

- [ ] `p1` - **ID**: `cpt-insightspec-contract-zulip-proxy-source`

**Direction**: required from client (the proxy is a server, the connector is a client).

**Protocol/Format**: HTTPS or HTTP, Bearer auth, JSON responses; two endpoints — `/api/users`
and `/api/messages`.

**Compatibility**: The connector depends on stable shapes for `users` (JSON object with `users:
[…]`) and `messages` (JSON object with `messages: […]` and `nextCursor` field for pagination).
Breaking changes to either shape will require connector updates; minor additive changes are
tolerated by `additionalProperties: true` on the InlineSchemaLoader.

**Implementation note**: See [DESIGN.md](./DESIGN.md) for the exact manifest layout, pagination
strategy, and incremental-sync configuration.

## 8. Use Cases

### UC-001 Refresh Zulip User Directory

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-zulip-proxy-refresh-users`

**Actor**: `cpt-insightspec-actor-zulip-proxy-operator`

**Preconditions**:
- K8s Secret exists with valid `zulip_proxy_base_url` and `zulip_proxy_api_key`.
- The proxy is reachable from the ingestion cluster.

**Main Flow**:
1. Argo workflow triggers the connector's `users` stream.
2. The connector issues `GET /api/users` against `{base_url}` with `Authorization: Bearer
   {api_key}`, paginated by `limit`/`offset`.
3. The proxy returns one page of user records under `{"users": […]}`.
4. The connector emits each user record stamped with `tenant_id`, `source_id`, `unique_key`.
5. Bronze persists rows into `bronze_zulip_proxy.users`.

**Postconditions**:
- `bronze_zulip_proxy.users` reflects the current Zulip directory at the time of sync.
- Identity Manager picks up new/changed user emails on the next Silver run.

**Alternative Flows**:
- **401 Unauthorized**: connector fails the run with a credential-rotation hint; operator rotates
  the Secret and re-runs.
- **5xx transient**: connector retries with backoff; if the bound is exhausted, the run fails and
  is rescheduled.

### UC-002 Collect Aggregated Message Activity

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-zulip-proxy-collect-messages`

**Actor**: `cpt-insightspec-actor-zulip-proxy-analyst`

**Preconditions**:
- The `users` stream has been collected at least once (for cross-stream identity stitching in
  Silver — not a hard runtime dependency).
- `zulip_proxy_start_date` is set; the previous sync's `created_at` checkpoint is either present
  or absent (first sync).

**Main Flow**:
1. Argo workflow triggers the connector's `messages` stream.
2. The connector issues `GET /api/messages?throttle={throttle_ms}&cursor={cursor}` against
   `{base_url}` with `Authorization: Bearer {api_key}`.
3. The proxy responds with `{"messages": [...], "nextCursor": "..."}` where each message is an
   aggregate of `(uniq, sender_id, count, created_at, …)`.
4. The connector emits each record stamped with `tenant_id`, `source_id`, `unique_key`.
5. Bronze persists rows into `bronze_zulip_proxy.messages`.
6. State is advanced to the largest observed `created_at`.

**Postconditions**:
- `bronze_zulip_proxy.messages` contains all aggregates with `created_at >= previous checkpoint`.
- Cursor state is persisted so the next sync resumes from the latest `created_at`.

**Alternative Flows**:
- **No new messages**: proxy returns an empty list; connector emits no records; state is
  unchanged.
- **Cursor truncation**: proxy returns `nextCursor: null` mid-window; connector treats the page
  as terminal and resumes from the highest `created_at` on the next run.

## 9. Acceptance Criteria

- [ ] On a fresh tenant, the connector populates `bronze_zulip_proxy.users` and
  `bronze_zulip_proxy.messages` end-to-end with `tenant_id`, `source_id`, `unique_key`.
- [ ] A repeated sync with an unchanged proxy produces zero new Silver rows after dedup.
- [ ] A 401 response from the proxy fails the run with an operator-actionable log line that names
  the connector and the source-id.
- [ ] Insight ingestion cluster holds no Zulip primary credentials anywhere on disk; only the
  Bearer token and proxy URL.
- [ ] `/check-dbt-conventions` passes for the connector's dbt models (engine, order_by,
  append-only sync, RMT promotion).
- [ ] `cfs validate` passes for PRD, DESIGN, FEATURE in this folder.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Zulip Proxy service | External service operated by the tenant; required for `users` and `messages` collection. | p1 |
| K8s Secret discovery | `connect.sh` discovers Bearer token + proxy URL via labels and annotations on the Secret. | p1 |
| Identity Manager | Resolves `email` → canonical `person_id` for cross-source analytics. | p1 |
| Bronze ingestion platform | Persists connector output as `bronze_zulip_proxy.users` and `bronze_zulip_proxy.messages`. | p1 |
| `promote_bronze_to_rmt` macro | Promotes plain `MergeTree` bronze tables to RMT after first dbt run; per ADR-0002. | p1 |
| Sibling `zulip` connector spec | Defines the canonical Bronze schemas this connector shares. | p2 |

## 11. Assumptions

- The proxy is the single, authoritative source of aggregated Zulip activity for the tenant; no
  parallel direct-API connector competes for the same Silver targets.
- The proxy preserves stable `id` values for users across runs (the user's Zulip-side primary
  key).
- The proxy preserves stable `uniq` values for message aggregates (used to compute `unique_key`).
- The proxy is reachable from the ingestion cluster network. Network reachability is the
  operator's responsibility, not the connector's.
- The proxy never returns message body content. The connector trusts this contract and does not
  add a content-stripping filter.
- The connector reuses the same Bronze schemas described in
  [../zulip/zulip.md](../../zulip/zulip.md), with the addition of `tenant_id`, `source_id`,
  `unique_key` for Insight's universal record stamping.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Proxy outage or maintenance window | Sync runs fail; downstream metrics stale until proxy returns | Argo retries with exponential backoff; operator dashboards monitor proxy uptime |
| Bearer token rotation drifts from Secret refresh | Connector fails with 401 until operator updates the Secret | Connector surfaces 401 as a distinct failure mode; operator runbook covers rotation |
| Proxy aggregation bucket changes (e.g. day → hour) | `created_at` semantics change without schema break — `count` interpretation shifts in Silver | Document the bucket in DESIGN.md §"Source Collection Strategy"; require the proxy to publish a versioned bucket descriptor before changing it |
| Proxy accidentally exposes message body content | Compliance boundary violated | Connector schema and identity-inputs models do NOT reference content fields; content is not promoted to Silver even if leaked into Bronze |
| Proxy `nextCursor` semantics ambiguous on empty pages | Cursor state may stall | The connector treats a missing/null `nextCursor` as terminal; the highest observed `created_at` becomes the resume anchor |
| Schema drift in `users`/`messages` payloads | Silver models break on missing columns | `InlineSchemaLoader` uses `additionalProperties: true`; Silver models alias and coalesce, not strict-select |
