# DESIGN — Zendesk Connector

> Version 2.0 — June 2026
> Issue: INSIGHT-459
> Based on: [PRD.md](./PRD.md), [`zendesk.md`](../zendesk.md), [Connector Framework DESIGN](../../../../domain/connector/specs/DESIGN.md), [Support Domain README](../../README.md)

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
  - [3.7 Database Schemas & Tables](#37-database-schemas--tables)
- [4. Additional Context](#4-additional-context)
  - [Audit fan-out hardening (review #1304)](#audit-fan-out-hardening-review-1304)
  - [API Details](#api-details)
  - [Field Mapping to Bronze Schema](#field-mapping-to-bronze-schema)
  - [Collection Strategy](#collection-strategy)
  - [Identity Resolution Details](#identity-resolution-details)
  - [Phase 2 Design Notes](#phase-2-design-notes)
  - [Incremental Sync Cursor and Datetime Format](#incremental-sync-cursor-and-datetime-format)
  - [Rate Limit Budget](#rate-limit-budget)
- [5. Traceability](#5-traceability)
- [6. Non-Applicability Statements](#6-non-applicability-statements)

<!-- /toc -->

---

## 1. Architecture Overview

### 1.1 Architectural Vision

The Zendesk connector is an Airbyte declarative (nocode) YAML manifest that extracts ticket snapshots, CSAT ratings, and agent directory data from the Zendesk REST API v2. It writes all data to per-source Bronze tables in ClickHouse, following the unified support domain Bronze schema defined in [`zendesk.md`](../zendesk.md).

Five streams ship:
1. `support_tickets` — incremental, via `GET /api/v2/incremental/tickets.json` with `metric_sets` sideload
2. `support_ticket_ids` — slim key-only parent (incremental, same export endpoint, `ticket_id` + `updated_at` only, no `metadata`) that fans out the audit substream while keeping the CDK parent-record cache tiny
3. `support_agents` — full refresh, via `GET /api/v2/users?role[]=agent&role[]=admin`
4. `zendesk_satisfaction_ratings` — incremental, via `GET /api/v2/satisfaction_ratings`
5. `support_ticket_events` — per-ticket Ticket Audits, via `GET /api/v2/tickets/{id}/audits` behind a `SubstreamPartitionRouter` over `support_ticket_ids`

Silver (`zendesk__support_event`, `class_support_activity`, shared dims) and Gold (`support_bullet_rows` → `support_person_period` → `support_company_stats`) are delivered.

`support_ticket_events` is SHIPPED; `zendesk_ticket_ext` (custom fields) remains Phase 2 — its schema is locked in `zendesk.md` and the Bronze DDL but not implemented in the connector manifest until Phase 2.

The declarative approach is consistent with all other connectors in the project. Authentication is via HTTP Basic Auth (`{email}/token:{api_token}` Base64-encoded). The manifest version is pinned to `7.0.4` to match the current platform CDK.

Every emitted record includes `insight_tenant_id`, `insight_source_id`, `unique_key`, `data_source`, and `collected_at` injected via `AddFields` transformations — standard practice per Connector Framework spec §4.6.

### 1.2 Architecture Drivers

**PRD Reference**: [PRD.md](./PRD.md)

#### Functional Drivers

| Requirement ID | Design Response |
|----------------|-----------------|
| `cpt-zendeskspec-fr-ticket-extraction` | `DeclarativeStream support_tickets` with `DatetimeBasedCursor` on `updated_at`; `GET /api/v2/incremental/tickets.json` with `?include=metric_sets` |
| `cpt-zendeskspec-fr-timing-both-variants` | `AddFields` transformations extract both `.business` and `.calendar` values from sideloaded `metric_set`; stored as four separate Int64 fields |
| `cpt-zendeskspec-fr-ticket-metadata` | Full API response stored as `metadata` String field via `DpathExtractor` on the full record |
| `cpt-zendeskspec-fr-agent-extraction` | `DeclarativeStream support_agents` with full refresh; `GET /api/v2/users?role[]=agent&role[]=admin`. Group-name resolution (`GET /api/v2/groups`) deferred to Phase 2 — `group_name` is NULL in Phase 1 |
| `cpt-zendeskspec-fr-ratings-extraction` | `DeclarativeStream zendesk_satisfaction_ratings` with `DatetimeBasedCursor` on `updated_at`; `GET /api/v2/satisfaction_ratings` |
| `cpt-zendeskspec-fr-collection-runs` | Connector-generated stream; written via a custom Python component or deferred to platform sync logs |
| `cpt-zendeskspec-fr-deduplication` | Primary keys defined per stream; ClickHouse `ReplacingMergeTree(_version)` handles upsert deduplication |
| `cpt-zendeskspec-fr-tenant-context` | `AddFields` transformation injects `tenant_id`, `source_id`, `unique_key`, `data_source`, `collected_at` on every record |
| `cpt-zendeskspec-fr-incremental-sync` | `DatetimeBasedCursor` with `cursor_field: updated_at`; Airbyte platform persists state between runs |
| `cpt-zendeskspec-fr-identity-resolution` | `email` field extracted from `support_agents`; no connector-level resolution — delegated to Identity Manager in Silver step 2 |

#### NFR Allocation

| NFR ID | Design Response | Verification |
|--------|-----------------|-------------|
| `cpt-zendeskspec-nfr-freshness` | Declarative manifest + daily cron schedule | Monitor `support_collection_runs.completed_at` |
| `cpt-zendeskspec-nfr-rate-limits` | Explicit RATE_LIMITED error_handler (429/503 + `Retry-After`) on the `support_ticket_events` audits substream (the most rate-limit-prone stream); other streams rely on the CDK default error handler / `Retry-After` | Zero unhandled HTTP 429 in production |
| `cpt-zendeskspec-nfr-utc` | Zendesk API returns ISO 8601 UTC timestamps; stored as-is in Bronze | Schema test: zero non-UTC timestamps |
| `cpt-zendeskspec-nfr-declarative` | YAML manifest; `validate-strict` in CI | Pipeline gate |

### 1.3 Architecture Layers

```text
┌─────────────────────────────────────────────────────────────────────┐
│  Orchestrator (Argo CronWorkflow: zendesk-default-sync)              │
│  (triggers Zendesk connector sync → dbt Silver run)                  │
└─────────────────────────────┬───────────────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────────────┐
│  Zendesk Connector (Airbyte DeclarativeSource YAML manifest)         │
│  ├── support_tickets stream                                          │
│  │   └── incremental: GET /api/v2/incremental/tickets.json          │
│  │       ?start_time={unix_ts}&include=metric_sets                   │
│  │       cursor: updated_at (DatetimeBasedCursor)                    │
│  ├── support_ticket_ids stream (slim key-only parent)               │
│  │   └── incremental: GET /api/v2/incremental/tickets.json          │
│  │       emits ticket_id + updated_at only (no metadata)            │
│  ├── support_agents stream                                           │
│  │   └── full refresh: GET /api/v2/users?role[]=agent&role[]=admin   │
│  │       group names: GET /api/v2/groups (Phase 2)                   │
│  ├── zendesk_satisfaction_ratings stream                            │
│  │   └── incremental: GET /api/v2/satisfaction_ratings               │
│  │       ?start_time={unix_ts}&sort_by=updated_at                    │
│  │       cursor: updated_at (DatetimeBasedCursor)                    │
│  └── support_ticket_events stream                                   │
│      └── SubstreamPartitionRouter over support_ticket_ids           │
│          GET /api/v2/tickets/{id}/audits (incremental_dependency)    │
└─────────────────────────────┬───────────────────────────────────────┘
                              │ Airbyte Protocol (RECORD, STATE, LOG)
┌─────────────────────────────▼───────────────────────────────────────┐
│  Bronze Tables (ClickHouse — ReplacingMergeTree)                     │
│  support_tickets, support_ticket_ids, support_agents,               │
│  zendesk_satisfaction_ratings, support_ticket_events                 │
│  (support_collection_runs — DE-SCOPED, not emitted)                 │
└─────────────────────────────┬───────────────────────────────────────┘
                              │ dbt (tag:zendesk+)
┌─────────────────────────────▼───────────────────────────────────────┐
│  Silver: zendesk__support_event, class_support_activity,            │
│          dim_support_agent, dim_support_ticket                       │
│  Gold:   support_bullet_rows → support_person_period →              │
│          support_company_stats                                       │
└──────────────────────────────────────────────────────────────────────┘
```

| Layer | Responsibility | Technology |
|-------|---------------|------------|
| Orchestration | Trigger, schedule, state management, dbt run | Argo CronWorkflow (`zendesk-default-sync`) |
| Collection | REST pagination, cursor management, retry, field injection, audit substream fan-out | Airbyte DeclarativeSource (YAML manifest) |
| Transformation | `AddFields` for tenant/source/key injection; timing field extraction | Declarative transformations + `CustomTransformation` |
| Storage | Upsert to Bronze tables | ClickHouse `ReplacingMergeTree(_version)` |
| Silver | `zendesk__support_event` (audit explosion + actor-classification), `class_support_activity` rollup, shared dims, identity resolution | dbt models (`tag:zendesk+`) — **delivered** |
| Gold | `support_bullet_rows` → `support_person_period` → `support_company_stats` (CH views) | migration `20260611000000_support-bullet-rows.sql` — **delivered** |

---

## 2. Principles & Constraints

### 2.1 Design Principles

#### Bronze-Only Output

- [ ] `p1` - **ID**: `cpt-zendeskspec-principle-bronze-only`

The Zendesk connector writes exclusively to `support_*` and `zendesk_*` Bronze tables via the declarative YAML manifest. No Silver or Gold layer logic exists in the connector. Cross-source unification into `class_support_activity` Silver and identity resolution (`email` → `person_id`) are responsibilities of downstream pipeline stages.

#### Incremental by Default

- [ ] `p1` - **ID**: `cpt-zendeskspec-principle-incremental`

`support_tickets` and `zendesk_satisfaction_ratings` use `DatetimeBasedCursor` on `updated_at`. Full collection is the degenerate case of an incremental run with no prior cursor (controlled by `start_date` config). `support_agents` is full-refresh on every run because the agent roster is small (hundreds of records) and does not expose a reliable incremental cursor.

#### Declarative-First

- [ ] `p1` - **ID**: `cpt-zendeskspec-principle-declarative-first`

Consistent with project convention, the Zendesk connector uses a nocode YAML manifest (Airbyte DeclarativeSource). Features that cannot be expressed declaratively (e.g., dynamic group name lookup) use `CustomTransformation` components rather than falling back to a full CDK Python implementation.

#### Fault Tolerance

- [ ] `p2` - **ID**: `cpt-zendeskspec-principle-fault-tolerance`

A partial sync that extracts most tickets is preferable to a run that halts on the first non-fatal error. HTTP 404 for missing resources, HTTP 429 with retry, and transient 5xx errors are handled gracefully. Fatal errors (401 authentication failure) halt the run immediately with a clear error message.

### 2.2 Constraints

#### Zendesk REST API v2

- [ ] `p1` - **ID**: `cpt-zendeskspec-constraint-api-version`

The connector targets Zendesk REST API v2 exclusively. There is no v3 for Zendesk — v2 is the stable public API. The incremental export endpoint (`/api/v2/incremental/tickets.json`) requires Zendesk Suite or equivalent plan.

#### Airbyte Declarative Manifest Compliance

- [ ] `p1` - **ID**: `cpt-zendeskspec-constraint-airbyte-cdk`

The connector MUST be a valid Airbyte DeclarativeSource YAML manifest at version `7.0.4`. It MUST emit valid Airbyte Protocol messages. Every emitted record MUST include `tenant_id`.

#### Rate Limit Budget

- [ ] `p1` - **ID**: `cpt-zendeskspec-constraint-rate-limit`

Zendesk rate limits: 200 req/min (Basic), 400 req/min (Suite Team), 700 req/min (Suite Growth and above). The connector MUST respect `Retry-After` headers on HTTP 429. Phase 2 `support_ticket_events` stream requires careful rate budget planning (one call per ticket).

#### No Silver Layer Logic

- [ ] `p1` - **ID**: `cpt-zendeskspec-constraint-no-silver`

The connector writes only to Bronze tables. `satisfaction_score` is NOT computed or backfilled onto `support_tickets` by the connector — Silver layer joins `zendesk_satisfaction_ratings` on `ticket_id` to derive this value.

---

## 3. Technical Architecture

### 3.1 Domain Model

**Core Entities**:

| Entity | Description | Maps To |
|--------|-------------|---------|
| `ZendeskInstance` | Connection config: subdomain, email, API token, start_date | Connector config (spec section) |
| `ZendeskTicket` | Ticket with current state + sideloaded `metric_set` | `support_tickets` |
| `ZendeskUser` | Agent/admin with role, group, email | `support_agents` |
| `ZendeskSatisfactionRating` | CSAT rating event: score, comment, reason | `zendesk_satisfaction_ratings` |
| `ZendeskGroup` | Team/department containing agents | Resolved into `group_name` at collection time |
| `ZendeskAudit` | Per-ticket event batch (Phase 2) | `support_ticket_events` |
| `ZendeskCustomField` | Per-ticket custom field value (Phase 2) | `zendesk_ticket_ext` |

**Entity Relationships**:
```
ZendeskTicket ──── assignee_id ───► ZendeskUser (support_agents)
ZendeskTicket ──── group_id ──────► ZendeskGroup
ZendeskTicket ──── ticket_id ──────► ZendeskSatisfactionRating (1:many)
ZendeskUser ────── default_group_id ► ZendeskGroup
ZendeskAudit ───── ticket_id ──────► ZendeskTicket [Phase 2]
```

### 3.2 Component Model

```
connector.yaml (Airbyte DeclarativeSource)
├── spec            — connection_specification with required fields
├── check           — CheckStream on support_agents (cheapest endpoint)
├── streams
│   ├── support_tickets              — incremental, DatetimeBasedCursor
│   ├── support_ticket_ids           — incremental, slim key-only audit parent (no metadata)
│   ├── support_agents               — full refresh
│   ├── zendesk_satisfaction_ratings — incremental, DatetimeBasedCursor
│   └── support_ticket_events        — SubstreamPartitionRouter over support_ticket_ids
│                                       (incremental_dependency; one row per audit)
└── metadata        — autoImportSchema flags per stream
```

**Check stream rationale**: `check` is a `CheckStream` against `support_agents` — the CDK fetches the first page of `GET /api/v2/users?role[]=agent&role[]=admin` and reports SUCCEEDED if credentials are valid. (A dedicated single-record `/users/me` check stream would be cheaper but is not currently defined.)

### 3.3 API Contracts

**Authentication**: HTTP Basic Auth
```
Authorization: Basic base64("{email}/token:{api_token}")
```

**Ticket incremental export**:
```
GET https://{subdomain}.zendesk.com/api/v2/incremental/tickets.json
  ?start_time={unix_timestamp}
  &include=metric_sets
  &per_page=1000                   (max per page; server-side cursor)
```
Response: `{ "tickets": [...], "next_page": "...", "after_cursor": "...", "end_of_stream": false }`

Cursor advance: Zendesk returns an opaque `after_cursor` (Base64-encoded JSON) as the next page token. When `end_of_stream = true`, save the returned `end_time` as the next run's `start_time`.

**Agent list**:
```
GET https://{subdomain}.zendesk.com/api/v2/users
  ?role[]=agent&role[]=admin
  &per_page=100
  &page[after]={cursor}            (cursor-based pagination)
```
Response: `{ "users": [...], "meta": {"after_cursor": "...", "has_more": true} }`

**Satisfaction ratings**:
```
GET https://{subdomain}.zendesk.com/api/v2/satisfaction_ratings
  ?start_time={unix_timestamp}
  &sort_by=updated_at
  &sort_order=asc
  &per_page=100
```
Response: `{ "satisfaction_ratings": [...], "next_page": "..." }`

**Group list** (startup lookup for `group_name` resolution):
```
GET https://{subdomain}.zendesk.com/api/v2/groups
  ?per_page=100
```
Response: `{ "groups": [{"id": 123, "name": "Tier 1 Support"}, ...] }`

### 3.4 Internal Dependencies

| Component | Dependency |
|-----------|-----------|
| `connector.yaml` | Airbyte DeclarativeSource CDK v7.0.4 |
| `descriptor.yaml` | Included in `insight-toolbox` image; read by reconcile loop |
| Bronze DDL | Applied by Airbyte ClickHouse destination on first run |
| dbt Silver models | `dbt/zendesk__support_activity.sql` — consumes Bronze tables (planned) |

### 3.5 External Dependencies

| Dependency | Version | Notes |
|-----------|---------|-------|
| Zendesk REST API | v2 (stable) | No versioning changes expected |
| Airbyte DeclarativeSource CDK | 7.0.4 (pinned) | Upgrade requires manifest syntax review |
| ClickHouse | ≥ 23.x | `ReplacingMergeTree` + JSON functions for `metadata` field |

### 3.6 Interactions & Sequences

**Incremental sync run (steady state)**:

```
Argo CronWorkflow
  │
  ├─ 1. Read Airbyte state from platform (cursor for tickets and ratings)
  │
  ├─ 2. support_agents stream (full refresh)
  │      GET /api/v2/users?role[]=agent&role[]=admin (paginated)
  │      [Phase 2] GET /api/v2/groups (startup) for group_name resolution
  │      → RECORD messages → ClickHouse support_agents
  │
  ├─ 3. support_tickets stream (incremental)
  │      GET /api/v2/incremental/tickets.json?start_time={cursor}&include=metric_sets
  │      Paginate until end_of_stream = true
  │      → RECORD messages → ClickHouse support_tickets
  │      → STATE message (updated ticket cursor)
  │
  ├─ 4. zendesk_satisfaction_ratings stream (incremental)
  │      GET /api/v2/satisfaction_ratings?start_time={cursor}
  │      Paginate until no next_page
  │      → RECORD messages → ClickHouse zendesk_satisfaction_ratings
  │      → STATE message (updated ratings cursor)
  │
  └─ 5. Airbyte platform persists STATE
         Argo triggers dbt Silver run (tag:zendesk+)
```

**First run sequence** (no prior state):
- `start_time` = Unix timestamp derived from `start_date` config (default: 90 days ago)
- Full historical tickets and ratings collected up to `end_of_stream`
- Can take several hours for large accounts; state checkpointed per page

### 3.7 Database Schemas & Tables

#### ClickHouse DDL: `support_tickets`

```sql
CREATE TABLE IF NOT EXISTS bronze_zendesk.support_tickets
(
    insight_source_id                      String,
    ticket_id                              String,
    subject                                Nullable(String),
    status                                 String,
    priority                               Nullable(String),
    ticket_type                            Nullable(String),
    assignee_id                            Nullable(String),
    group_id                               Nullable(String),
    requester_id                           Nullable(String),
    organization_id                        Nullable(String),
    created_at                             DateTime64(3),
    updated_at                             DateTime64(3),
    solved_at                              Nullable(DateTime64(3)),
    first_reply_time_seconds               Nullable(Int64),
    first_reply_time_calendar_seconds      Nullable(Int64),
    full_resolution_time_seconds           Nullable(Int64),
    full_resolution_time_calendar_seconds  Nullable(Int64),
    satisfaction_score                     Nullable(String),   -- NULL in Phase 1
    tags                                   Nullable(String),
    metadata                               String,
    data_source                            String DEFAULT 'insight_zendesk',
    _version                               UInt64,
    -- Airbyte standard fields
    _ab_source_file_url                    Nullable(String),
    insight_tenant_id                      String,
    unique_key                             String
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY (insight_source_id, ticket_id)
SETTINGS index_granularity = 8192;

ALTER TABLE bronze_zendesk.support_tickets
    ADD INDEX idx_support_ticket_assignee  (assignee_id, data_source) TYPE minmax GRANULARITY 4,
    ADD INDEX idx_support_ticket_updated   (updated_at) TYPE minmax GRANULARITY 4,
    ADD INDEX idx_support_ticket_status    (status, data_source) TYPE set(100) GRANULARITY 4;
```

#### ClickHouse DDL: `support_agents`

```sql
CREATE TABLE IF NOT EXISTS bronze_zendesk.support_agents
(
    insight_source_id  String,
    agent_id           String,
    email              String,
    display_name       Nullable(String),
    role               Nullable(String),
    group_id           Nullable(String),
    group_name         Nullable(String),
    is_active          Int64 DEFAULT 1,
    collected_at       DateTime64(3),
    data_source        String DEFAULT 'insight_zendesk',
    _version           UInt64,
    insight_tenant_id  String,
    unique_key         String
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY (insight_source_id, agent_id)
SETTINGS index_granularity = 8192;

ALTER TABLE bronze_zendesk.support_agents
    ADD INDEX idx_support_agent_email (email) TYPE set(0) GRANULARITY 4;
```

#### ClickHouse DDL: `zendesk_satisfaction_ratings`

```sql
CREATE TABLE IF NOT EXISTS bronze_zendesk.zendesk_satisfaction_ratings
(
    insight_source_id  String,
    rating_id          String,
    ticket_id          String,
    requester_id       Nullable(String),
    assignee_id        Nullable(String),
    group_id           Nullable(String),
    score              Nullable(String),
    comment            Nullable(String),
    reason             Nullable(String),
    created_at         DateTime64(3),
    updated_at         DateTime64(3),
    data_source        String DEFAULT 'insight_zendesk',
    _version           UInt64,
    insight_tenant_id  String,
    unique_key         String
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY (insight_source_id, rating_id)
SETTINGS index_granularity = 8192;

ALTER TABLE bronze_zendesk.zendesk_satisfaction_ratings
    ADD INDEX idx_zendesk_rating_ticket   (insight_source_id, ticket_id, data_source) TYPE minmax GRANULARITY 4,
    ADD INDEX idx_zendesk_rating_updated  (updated_at) TYPE minmax GRANULARITY 4,
    ADD INDEX idx_zendesk_rating_assignee (assignee_id, data_source) TYPE set(0) GRANULARITY 4;
```

#### ClickHouse DDL: `support_ticket_ids`

Slim key-only parent for the audit substream — emits `ticket_id` + `updated_at` only (no `metadata`).

```sql
CREATE TABLE IF NOT EXISTS bronze_zendesk.support_ticket_ids
(
    ticket_id          String,
    updated_at         DateTime64(3),
    collected_at       DateTime64(3),
    data_source        String DEFAULT 'insight_zendesk',
    tenant_id          String,
    source_id          String,
    unique_key         String
)
ENGINE = ReplacingMergeTree
ORDER BY unique_key
SETTINGS index_granularity = 8192;
```

#### ClickHouse DDL: `support_ticket_events`

One row per AUDIT; the raw `events[]` array is stored as a JSON string. Per-event explosion + actor-classification happen in Silver (`zendesk__support_event`).

```sql
CREATE TABLE IF NOT EXISTS bronze_zendesk.support_ticket_events
(
    tenant_id          String,
    source_id          String,
    unique_key         String,            -- = audit id
    collected_at       DateTime64(3),
    data_source        String DEFAULT 'insight_zendesk',
    audit_id           String,
    ticket_id          String,
    author_id          Nullable(String),  -- audit.author_id; NULL for system events
    created_at         DateTime64(3),      -- audit.created_at
    events             String              -- audit.events[] as a JSON string
)
ENGINE = ReplacingMergeTree
ORDER BY unique_key
SETTINGS index_granularity = 8192;
```

---

## 4. Additional Context

### Audit fan-out hardening (review #1304)

The `support_ticket_events` audit substream fans out one `GET /api/v2/tickets/{id}/audits` call per ticket. Several hardening measures protect it against the failure modes seen in the jira/confluence connectors:

- **Slim key-only parent (`support_ticket_ids`)**: the substream's `SubstreamPartitionRouter` reads from `support_ticket_ids` (which emits only `ticket_id` + `updated_at`, no `metadata`) rather than the full `support_tickets` stream. This keeps the CDK parent-record cache tiny and avoids the parent-cache bloat that caused the jira/confluence silent-hang.
- **`incremental_dependency: true`**: the substream re-fetches audits only for tickets present in the parent's current incremental window, so the cursor on the parent governs audit refresh.
- **`concurrency_level.default_concurrency: 4`** at the top level — the audit fan-out must run concurrently; this honours jira's documented "must be ≥ 2" rule to avoid a deadlocked single-partition scheduler.
- **error_handler**: IGNORE on HTTP 404 (deleted tickets still surface in the incremental export, but their audits 404), and RATE_LIMITED on HTTP 429 / 503 with `Retry-After` backoff.
- **`lookback_window: P1D`** on the incremental cursors (`support_tickets`, `support_ticket_ids`, `zendesk_satisfaction_ratings`) — boundary-skip protection against records landing exactly on the cursor edge (cf. jira #1316).

### API Details

#### Incremental Ticket Export vs Full Scan

The incremental export endpoint (`/api/v2/incremental/tickets.json?start_time=`) is the preferred collection strategy:
- Returns up to 1000 tickets per page (vs 100 for `GET /api/v2/tickets`)
- Uses a server-side cursor (`after_cursor`) that is stable across requests — no risk of missing tickets due to pagination drift
- Includes a `metric_sets` sideload to avoid N+1 calls for timing fields
- Returns `end_of_stream: true` when the consumer is caught up to the present; cursor should advance to `end_time` for the next run

**Cursor format**: `start_time` is a Unix timestamp (seconds). The CDK `DatetimeBasedCursor` supports `datetime_format: "%s"` for Unix timestamp output.

#### Pagination Strategies

| Stream | Pagination type | Page size |
|--------|----------------|-----------|
| `support_tickets` | Server cursor (`after_cursor` in response body) | 1000 (server-enforced max) |
| `support_agents` | Cursor-based (`page[after]` query param, `meta.after_cursor` in response) | 100 |
| `zendesk_satisfaction_ratings` | Next-page link (`next_page` URL in response body) | 100 |
| `groups` (startup lookup) | Offset-based (`page`, `per_page`) | 100 |

#### group_name Resolution — Phase 2

`support_agents.default_group_id` is a numeric Zendesk group ID. **Phase 1 stores `group_name` as NULL.** Group-name resolution is deferred to Phase 2; when implemented, the connector will:
1. Fetch all groups via `GET /api/v2/groups` at stream initialization
2. Build an in-memory `{group_id: group_name}` map
3. Apply a `CustomTransformation` (or `RecordTransformation`) to populate `group_name` from the map for each agent record

If the groups endpoint returns HTTP 403 (insufficient scope), `group_name` stays NULL for all agents — sync continues without failure.

### Field Mapping to Bronze Schema

#### `support_tickets` field mapping

| Bronze field | Zendesk API path | Notes |
|-------------|-----------------|-------|
| `ticket_id` | `ticket.id` | Cast to String |
| `subject` | `ticket.subject` | NULL if empty |
| `status` | `ticket.status` | Direct mapping (Zendesk values match unified schema) |
| `priority` | `ticket.priority` | NULL if not set |
| `ticket_type` | `ticket.type` | NULL if not set |
| `assignee_id` | `ticket.assignee_id` | Cast to String; NULL if unassigned |
| `group_id` | `ticket.group_id` | Cast to String; NULL if unassigned |
| `requester_id` | `ticket.requester_id` | Cast to String |
| `organization_id` | `ticket.organization_id` | Cast to String; NULL if not set |
| `created_at` | `ticket.created_at` | ISO 8601 → DateTime64(3) |
| `updated_at` | `ticket.updated_at` | ISO 8601 → DateTime64(3); cursor field |
| `solved_at` | `ticket.metric_set.solved_at` | ISO 8601 → DateTime64(3); NULL if not solved |
| `first_reply_time_seconds` | `ticket.metric_set.reply_time_in_minutes.business × 60` | NULL if no reply |
| `first_reply_time_calendar_seconds` | `ticket.metric_set.reply_time_in_minutes.calendar × 60` | NULL if no reply |
| `full_resolution_time_seconds` | `ticket.metric_set.full_resolution_time_in_minutes.business × 60` | NULL if unresolved |
| `full_resolution_time_calendar_seconds` | `ticket.metric_set.full_resolution_time_in_minutes.calendar × 60` | NULL if unresolved |
| `satisfaction_score` | NULL (Phase 1) | Phase 2: derived from `zendesk_satisfaction_ratings` at Silver |
| `tags` | `ticket.tags` (array) | Joined as comma-separated string |
| `metadata` | Full `ticket` object as JSON | Stored via `AddFields` from raw record |

**Implementation note**: on the incremental export endpoint, `?include=metric_sets` embeds the metric set directly in each ticket as a singular `metric_set` object (verified against `constructor-tech.zendesk.com`). No separate-array join or `CustomTransformation` is required — the `AddFields` transformations read `record.metric_set.*` directly during field extraction.

#### `zendesk_satisfaction_ratings` field mapping

| Bronze field | Zendesk API path | Notes |
|-------------|-----------------|-------|
| `rating_id` | `satisfaction_rating.id` | Cast to String |
| `ticket_id` | `satisfaction_rating.ticket_id` | Cast to String |
| `requester_id` | `satisfaction_rating.requester_id` | Cast to String |
| `assignee_id` | `satisfaction_rating.assignee_id` | Cast to String; NULL if not set |
| `group_id` | `satisfaction_rating.group_id` | Cast to String; NULL if not set |
| `score` | `satisfaction_rating.score` | `"good"` / `"bad"` / `"offered"` |
| `comment` | `satisfaction_rating.comment` | NULL if no comment |
| `reason` | `satisfaction_rating.reason` or `satisfaction_rating.reason_code` | NULL if not provided |
| `created_at` | `satisfaction_rating.created_at` | ISO 8601 → DateTime64(3) |
| `updated_at` | `satisfaction_rating.updated_at` | ISO 8601 → DateTime64(3); cursor field |

### Collection Strategy

#### Cursor Initialization

On first run, `start_time` for both incremental streams is derived from `config['start_date']` (YYYY-MM-DD string → Unix timestamp). Default: 90 days ago (`day_delta(-90, format='%s')`).

Operators setting `start_date: "2024-01-01"` get a full historical backfill from that date. No hard lower bound — Zendesk API returns empty results before the account creation date.

#### Cursor Advancement

`DatetimeBasedCursor` with `datetime_format: "%s"` (Unix timestamp):
- `cursor_field: updated_at` for both `support_tickets` and `zendesk_satisfaction_ratings`
- Cursor advances to the maximum `updated_at` seen in the current run
- Airbyte platform persists state; cursor survives process restarts

#### Support Agents — Full Refresh Decision

`support_agents` is full-refresh because:
1. The agent roster is small (typically hundreds, rarely thousands)
2. Zendesk does not expose a reliable incremental endpoint for users (`GET /api/v2/incremental/users.json` requires a paid Explore add-on on some plans)
3. Agent deactivation (soft delete) does not emit an event visible via cursor-based sync — full refresh ensures deactivated agents are captured with `is_active = 0`

### Identity Resolution Details

**Flow**:
```
Bronze: support_agents.email
  → Silver step 2: Identity Manager
  → person_id (canonical)
  → Propagated to zendesk_satisfaction_ratings.assignee_id via support_agents.agent_id join
```

**Email normalization**: performed in Silver step 2, not in the connector. Silver applies `lower(trim(email))` before lookup. The connector stores email as-is from the Zendesk API.

**Multi-instance disambiguation**: when a customer has two Zendesk instances (e.g., `acme-prod` and `acme-staging`), each has its own `insight_source_id`. The Identity Manager joins on `(email, insight_source_id)` — agents with the same email in two instances map to the same `person_id`.

### Phase 2 Design Notes

#### `support_ticket_events` Collection Strategy — IMPLEMENTED

The audit log stream is shipped. Key design decisions, as implemented:

1. **Per-ticket API calls**: `GET /api/v2/tickets/{id}/audits` — one call per ticket. For 500K tickets at 700 req/min, first run takes ~12 hours. Mitigated by:
   - Configurable `start_date` limits the set of tickets whose audits are fetched
   - The audit substream fans out only over tickets in the parent's current incremental window (`incremental_dependency: true`)
   - Airbyte `SubstreamPartitionRouter` over the slim key-only parent `support_ticket_ids` handles the per-ticket fan-out pattern (see "Audit fan-out hardening")

2. **Deduplication keys**:
   - Bronze `support_ticket_events.unique_key` = the **audit id** (one row per audit; the raw `events[]` array is stored as a JSON string).
   - Silver per-event key (`zendesk__support_event`, after the ARRAY JOIN explosion) = `zendesk-{audit_id}-{event.id}` — composite because Zendesk audit IDs are unique per audit but not per event within the audit.

3. **Cursor**: no direct cursor on audits. The parent `support_ticket_ids` incremental cursor controls which tickets have their audits refreshed — audits are re-fetched for all tickets in the current incremental window.

#### `zendesk_ticket_ext` Collection Strategy

Phase 2 adds custom fields extracted from `ticket.custom_fields[]`:
1. `GET /api/v2/ticket_fields` fetched at startup — builds `{field_id: {title, type}}` map
2. `custom_fields[]` array on each ticket record iterated; one row per non-null custom field
3. Implemented as a `SubstreamPartitionRouter` pattern or as a dedicated `CustomTransformation`

### Incremental Sync Cursor and Datetime Format

Zendesk uses Unix timestamps (seconds since epoch) for the incremental export endpoint:
```
GET /api/v2/incremental/tickets.json?start_time=1706745600
```

The `DatetimeBasedCursor` configuration:
```yaml
incremental_sync:
  type: DatetimeBasedCursor
  cursor_field: updated_at
  cursor_datetime_formats:
    - "%Y-%m-%dT%H:%M:%SZ"          # Zendesk returns ISO 8601
    - "%Y-%m-%dT%H:%M:%S.%fZ"       # with optional microseconds
  datetime_format: "%s"              # emit as Unix timestamp for start_time param
  start_datetime:
    type: MinMaxDatetime
    datetime: "{{ config.get('start_date') or day_delta(-90, format='%Y-%m-%d') }}"
    datetime_format: "%Y-%m-%d"
  end_datetime:
    type: MinMaxDatetime
    datetime: "{{ now_utc().strftime('%Y-%m-%dT%H:%M:%SZ') }}"
    datetime_format: "%Y-%m-%dT%H:%M:%SZ"
  start_time_option:
    type: RequestOption
    field_name: start_time
    inject_into: request_parameter
```

### Rate Limit Budget

Estimated API calls per run (steady state, daily incremental):

| Stream | Calls per run | Notes |
|--------|--------------|-------|
| `support_agents` groups lookup | 1–5 | One call per 100 groups; most accounts < 100 groups |
| `support_agents` users | 1–20 | One call per 100 agents; most support teams < 2000 agents |
| `support_tickets` | 1–10 | One call per 1000 tickets; daily delta rarely exceeds 10K tickets |
| `zendesk_satisfaction_ratings` | 1–5 | One call per 100 ratings; daily delta rarely exceeds 500 ratings |
| **Total (steady state)** | **~10–40** | Far below rate limit |

**First run budget** (90-day backfill, 50K tickets):
- `support_tickets`: ~50 calls (50 pages × 1000/page)
- `zendesk_satisfaction_ratings`: ~500 calls (500 pages × 100/page, assuming ~50K ratings)
- Still well within the 700 req/min limit for Business plan accounts

**Audit-stream first-run budget** (`support_ticket_events`, shipped; 50K tickets, no lookback limit):
- 50,000 calls for audit fetches alone
- At 700 req/min: ~71 minutes minimum
- Mitigated by default 90-day lookback (~5K–10K tickets for typical active accounts)
- The explicit RATE_LIMITED error_handler (429/503 + `Retry-After`) lives on this substream — it is the most rate-limit-prone stream because of the per-ticket fan-out; the other streams use the CDK defaults.

---

## 5. Traceability

| PRD Requirement | Design Decision | Location |
|----------------|-----------------|---------|
| `cpt-zendeskspec-fr-ticket-extraction` | `DeclarativeStream support_tickets` with `DatetimeBasedCursor` | §3.2, §4.2 |
| `cpt-zendeskspec-fr-timing-both-variants` | Four timing fields in Bronze DDL; extracted via `AddFields` | §3.7, §4.2 |
| `cpt-zendeskspec-fr-ticket-metadata` | `metadata` String field stores full JSON | §3.7 |
| `cpt-zendeskspec-fr-agent-extraction` | `DeclarativeStream support_agents` full refresh | §3.2, §3.3 |
| `cpt-zendeskspec-fr-group-name-resolution` | **Phase 2** — `group_name` NULL in Phase 1; startup `GET /api/v2/groups` + `CustomTransformation` when implemented | §4.2 |
| `cpt-zendeskspec-fr-ratings-extraction` | `DeclarativeStream zendesk_satisfaction_ratings` incremental | §3.2, §3.3 |
| `cpt-zendeskspec-fr-deduplication` | `ReplacingMergeTree(_version)` + primary keys | §3.7 |
| `cpt-zendeskspec-fr-tenant-context` | `AddFields` transformation on every stream | §1.1 |
| `cpt-zendeskspec-fr-incremental-sync` | `DatetimeBasedCursor` with `cursor_field: updated_at` | §4.4 |
| `cpt-zendeskspec-fr-identity-resolution` | `email` in `support_agents`; no connector-level resolution | §4.3 |
| `cpt-zendeskspec-nfr-rate-limits` | CDK default `BackoffStrategy`; `Retry-After` honoured | §4.5 |
| `cpt-zendeskspec-nfr-declarative` | YAML manifest; `validate-strict` in CI | §2.2 |

---

## 6. Non-Applicability Statements

| Requirement | Why Not Applicable |
|------------|-------------------|
| Server-side filtering / push | Connector is pull-only batch; Zendesk webhooks are out of scope |
| OAuth 2.0 as primary auth | API token (Basic Auth) is preferred for service accounts; OAuth is supported by spec but not the primary implementation path for Phase 1 |
| SLA Policy object extraction | Zendesk pre-computes timing on tickets (`metric_set`); no explicit SLA breach objects exist in the Zendesk API (unlike JSM's `/rest/servicedeskapi/request/{id}/sla`) |
| `satisfaction_score` backfill at collection | Deliberately deferred to Silver layer — connector stores NULL; Silver joins `zendesk_satisfaction_ratings` on `ticket_id` for the latest score |
| Multi-threaded / concurrent fetches | Phase 1 streams are independent and fast; concurrency adds complexity without measurable benefit for the estimated call volumes |
