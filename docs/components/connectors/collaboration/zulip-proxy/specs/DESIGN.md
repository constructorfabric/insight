# DESIGN — Zulip-Proxy Connector

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
- [4. Additional context](#4-additional-context)
  - [Source Collection Strategy](#source-collection-strategy)
  - [Incremental Strategy](#incremental-strategy)
  - [Identity Model](#identity-model)
  - [Pagination](#pagination)
  - [Throttle](#throttle)
  - [Idempotence](#idempotence)
  - [Run Logging and Observability](#run-logging-and-observability)
  - [Assumptions and Risks That Affect Implementation](#assumptions-and-risks-that-affect-implementation)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

The Zulip-Proxy connector is an Airbyte declarative (no-code) source that reads two pre-aggregated
endpoints from a tenant-controlled proxy and writes the data into the Insight Bronze layer with
the same record-stamping conventions used by every other connector in this repository
(`tenant_id`, `source_id`, `unique_key`). It deliberately mirrors the Bronze schema of the
direct-API Zulip connector spec so that Silver and Gold can be source-agnostic.

The connector itself is a manifest only — there is no Python code, no Docker image, and no
per-tenant configuration baked into the package. Everything that varies per deployment lives in
the K8s Secret consumed at sync time: the proxy URL, the Bearer token, the backfill anchor, and
the optional throttle hint.

### 1.2 Architecture Drivers

#### Functional Drivers

- Deliver the same Bronze schema (`zulip_users`, `zulip_messages`) as the existing Zulip Basic-
  Auth connector spec so Silver mappings can be reused without per-source branching.
- Support both the cold-start backfill (oldest available aggregate ≥ `zulip_proxy_start_date`)
  and steady-state incremental collection on the `created_at` cursor.
- Carry tenant and source stamps onto every emitted row so the universal `(tenant_id, source_id,
  unique_key)` join scope holds.

#### NFR Allocation

- **Freshness** is bounded by the operator-managed schedule (Argo workflow cadence × proxy
  refresh cadence), not by the connector's pull latency.
- **Credential blast radius** is bounded by the proxy: the connector holds only the Bearer
  token; it never sees Zulip's bot email or API key.
- **Idempotence** is bounded by `unique_key` determinism on both streams and by the
  `ReplacingMergeTree(_version=_airbyte_extracted_at)` engine assigned to bronze tables after the
  first `promote_bronze_to_rmt` run.

### 1.3 Architecture Layers

| Layer | What lives here | This connector's contribution |
|-------|------------------|-------------------------------|
| Source | The Zulip-proxy HTTP API | None — external |
| Ingest | Airbyte declarative source (this manifest) | `connector.yaml` (v7.0.4), `configured_catalog.json` |
| Bronze | `bronze_zulip_proxy.users` + `bronze_zulip_proxy.messages` | Schema declared inline in the manifest; written by the shared ClickHouse destination |
| Silver | `staging.zulip_proxy__*` dbt models | `zulip_proxy__bronze_promoted`, `zulip_proxy__users_snapshot`, `zulip_proxy__users_fields_history`, `zulip_proxy__identity_inputs`, `zulip_proxy__collab_chat_activity` |
| Identity | Identity Manager `identity_inputs` table | Fed by `zulip_proxy__identity_inputs` |
| Gold | Source-agnostic collaboration metrics | Reads from `class_collab_chat_activity` (no source-specific Gold) |

## 2. Principles & Constraints

### 2.1 Design Principles

#### Manifest-Driven Simplicity

- [ ] `p1` - **ID**: `cpt-insightspec-principle-zulip-proxy-manifest-driven`

The connector is described entirely by its declarative manifest. No Python, no Docker image, no
CDK extensions. Reason: the proxy's contract is small (two endpoints, Bearer auth, JSON), and
declarative sources are the cheapest path to a maintainable connector that other engineers can
read without learning a SDK.

#### Configurable Transport, Static Schema

- [ ] `p1` - **ID**: `cpt-insightspec-principle-zulip-proxy-config-transport`

Transport-level concerns (proxy URL, Bearer token, throttle hint, backfill anchor) MUST be
configurable per K8s Secret. Schema-level concerns (field names, types, primary keys) MUST be
static in the manifest. Reason: deployments differ in topology, not in the data they carry.

#### Pre-Aggregated Data, Never Content

- [ ] `p1` - **ID**: `cpt-insightspec-principle-zulip-proxy-no-content`

The connector ingests only pre-aggregated counts and the user directory. Even if the proxy
inadvertently exposes message body content, the connector's InlineSchemaLoader does not declare
content fields, and downstream Silver models never reference them.

### 2.2 Constraints

#### Sibling Schema Parity With Direct Zulip Connector

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-zulip-proxy-schema-parity`

The Bronze schemas declared by this connector MUST remain a strict superset of the canonical
Zulip Bronze schemas defined in `docs/components/connectors/collaboration/zulip/zulip.md`. The
"superset" is exactly the universal stamping fields (`tenant_id`, `source_id`, `unique_key`).
Reason: Silver layer cannot branch per transport.

#### Append-Only Bronze, RMT After First dbt Run

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-zulip-proxy-append-only-bronze`

Airbyte writes Bronze with `destinationSyncMode='append'` per
[ADR-0002](../../../../../domain/ingestion-data-flow/specs/ADR/0002-promote-bronze-to-rmt.md).
This connector MUST follow the convention: `zulip_proxy__bronze_promoted.sql` promotes
`bronze_zulip_proxy.users` and `bronze_zulip_proxy.messages` to
`ReplacingMergeTree(_airbyte_extracted_at)` ordered by `unique_key` on the first dbt run.

#### Bearer-Only Credential

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-zulip-proxy-bearer-only`

The connector MUST authenticate only with a Bearer token sourced from the K8s Secret field
`zulip_proxy_api_key`. It MUST NOT accept Zulip primary credentials. Reason: compliance
boundary.

## 3. Technical Architecture

### 3.1 Domain Model

The connector emits two record types into Bronze, both shaped by the proxy's response payloads:

- **User record** — a row in the Zulip realm's user directory (one row per user). Identifiers:
  `id` (Zulip internal), `uuid` (universally unique), `email` (identity anchor). State:
  `is_active`, `role`. Linkage: `recipient_id` (Zulip-internal recipient identifier).
- **Message-aggregate record** — one bucket of message counts attributed to a sender within an
  aggregation period. Identifiers: `uniq` (proxy-assigned primary key for the bucket),
  `sender_id` (foreign key to `users.id`). Measures: `count`. Time: `created_at` (period anchor,
  used as the incremental cursor).

The connector adds three universal fields to every record before emit: `tenant_id`, `source_id`,
`unique_key`.

### 3.2 Component Model

#### Declarative Stream Components

- [ ] `p1` - **ID**: `cpt-insightspec-component-zulip-proxy-declarative-streams`

##### Why this component exists

The connector's only logic is the Airbyte declarative manifest. Components are:

- `DeclarativeSource` (root) — `version: 7.0.4`.
- `CheckStream` — uses the `users` stream as the lightweight liveness probe.
- `definitions.linked.HttpRequester` — shared transport configuration: `url_base` rendered from
  `config['zulip_proxy_base_url']`, `BearerAuthenticator` with `api_token` from
  `config['zulip_proxy_api_key']`, `error_handler` covering 401/429/5xx with `Retry-After`.
- `definitions.linked.SimpleRetriever.paginator` — defines two pagination patterns:
  - Offset paginator for `users` (`limit`/`offset` request parameters).
  - Cursor paginator for `messages` (`cursor` request parameter, `nextCursor` response field,
    null-cursor stop condition).
- Two `DeclarativeStream` instances at root: `users` and `messages`.
- `transformations` per stream — `AddFields` injecting `tenant_id`, `source_id`, `unique_key`.
- `incremental_sync` on `messages` only — `DatetimeBasedCursor` with `cursor_field=created_at`,
  `start_datetime` from `config['zulip_proxy_start_date']`, end open.
- `concurrency_level: 1` — both streams hit the same proxy; concurrency would just amplify
  throttle pressure.
- `spec.connection_specification` — declared fields: `insight_tenant_id`, `insight_source_id`,
  `zulip_proxy_base_url`, `zulip_proxy_api_key` (`airbyte_secret: true`), `zulip_proxy_start_date`,
  `zulip_proxy_throttle_ms`.
- `metadata.autoImportSchema` — both streams enabled.

##### Responsibility scope

The manifest is responsible for transport, pagination, incremental state, record stamping, and
schema declaration. It is NOT responsible for: identity resolution (Identity Manager), Silver
shaping (`zulip_proxy__*` dbt models), Bronze→RMT promotion (`promote_bronze_to_rmt` macro), or
scheduling (Argo).

##### Responsibility boundaries

- The connector emits Bronze rows; everything else is downstream.
- The connector trusts the proxy to never return content fields; it does not validate the payload
  against an allowlist of fields.
- The connector does not write its own run-log table. Operational visibility is provided by
  Argo + Airbyte logs (see ADR-0006 in airbyte-toolkit specs).

##### Related components (by ID)

- `cpt-airbyte-toolkit-component-reconcile-engine` (registers the source/connection from the
  descriptor).
- `cpt-dataflow-component-bronze` (universal Bronze layer; receives append-only writes).
- `cpt-dataflow-principle-promote-bronze` (RMT promotion on first dbt run).

### 3.3 API Contracts

#### `GET /api/users`

- **Auth**: `Authorization: Bearer {zulip_proxy_api_key}`.
- **Query parameters**: `limit` (page size, default 100), `offset` (pagination anchor).
- **Response**: `{"users": [ { "id": int, "uuid": str, "email": str, "full_name": str, "role":
  int, "is_active": bool, "recipient_id": int }, … ] }`.
- **Pagination**: offset-incremented by 100 until the page is shorter than `limit` (no
  `nextCursor` field).
- **Errors**: 401 (token rotation), 5xx (proxy fault), 429 (proxy backpressure with
  `Retry-After`).

#### `GET /api/messages`

- **Auth**: same as users.
- **Query parameters**: `cursor` (opaque, returned by proxy in `nextCursor`), `throttle`
  (milliseconds; forwarded from `zulip_proxy_throttle_ms`).
- **Response**: `{"messages": [ { "uniq": str, "sender_id": int, "count": float, "created_at":
  "%Y-%m-%dT%H:%M:%S.%fZ" }, … ], "nextCursor": str|null }`.
- **Pagination**: opaque cursor; stop when `nextCursor` is missing or null.
- **Incremental sync**: the cursor field is `created_at`; the manifest passes
  `zulip_proxy_start_date` as the start anchor on first sync.

### 3.4 Internal Dependencies

| Dependency | How it is used | Reference |
|------------|----------------|-----------|
| `insight-airbyte-toolkit` | Discovers the K8s Secret, registers the Airbyte source and connection, runs the sync, hands off to dbt | `docs/components/airbyte-toolkit/specs/DESIGN.md` |
| `promote_bronze_to_rmt` dbt macro | Promotes `bronze_zulip_proxy.users` and `bronze_zulip_proxy.messages` to RMT | `src/ingestion/dbt/macros/promote_bronze_to_rmt.sql`, ADR-0002 |
| `identity_inputs_from_history` dbt macro | Builds `zulip_proxy__identity_inputs` from the SCD2 user history | `src/ingestion/dbt/macros/identity_inputs_from_history.sql` |
| `snapshot` / `fields_history` dbt macros | Standard SCD2 plumbing for the user directory | `src/ingestion/dbt/macros/` |

### 3.5 External Dependencies

- **Zulip Proxy service** — operator-managed; not part of this repo. The connector treats it as
  an opaque HTTP server with the contract documented in §3.3. Proxy uptime, aggregation
  semantics, and Bearer-token rotation are the operator's responsibility.

### 3.6 Interactions & Sequences

#### Users sync

- [ ] `p1` - **ID**: `cpt-insightspec-seq-zulip-proxy-users-sync`

```
Argo workflow
  → Airbyte job (source=zulip-proxy, stream=users)
     → HTTP GET {base_url}/api/users?limit=100&offset=0  [Bearer]
        ← {"users": [...]}
     → emit RECORD × N  (each with tenant_id, source_id, unique_key)
     → HTTP GET ?limit=100&offset=100
        ← {"users": [...]}
     → emit RECORD × N
     → (continue until page < limit)
     → STATE  (no cursor on full-refresh)
  → ClickHouse destination: APPEND into bronze_zulip_proxy.users
```

#### Messages sync (first run)

- [ ] `p1` - **ID**: `cpt-insightspec-seq-zulip-proxy-messages-sync`

```
Argo workflow
  → Airbyte job (source=zulip-proxy, stream=messages)
     → HTTP GET {base_url}/api/messages?throttle=5000  [Bearer]
        ← {"messages": [...], "nextCursor": "abc"}
     → emit RECORD × N + STATE(created_at=max(records.created_at))
     → HTTP GET ?throttle=5000&cursor=abc
        ← {"messages": [...], "nextCursor": "def"}
     → emit RECORD × N + STATE
     → HTTP GET ?throttle=5000&cursor=def
        ← {"messages": [...], "nextCursor": null}
     → emit RECORD × N + STATE (terminal)
  → ClickHouse destination: APPEND into bronze_zulip_proxy.messages
```

On the second and subsequent runs, the connector replays the latest persisted `created_at` into
`DatetimeBasedCursor.start_datetime` (handled by the CDK runtime; no manifest change needed).

### 3.7 Database schemas & tables

#### Table: `bronze_zulip_proxy.users`

Bronze table, populated by Airbyte with `destinationSyncMode='append'`, promoted to
`ReplacingMergeTree(_airbyte_extracted_at)` ordered by `(unique_key)` on the first dbt run.

| Column | Type | Description |
|--------|------|-------------|
| `id` | `Nullable(Float64)` | Zulip-internal numeric user ID. |
| `uuid` | `Nullable(String)` | Zulip-internal universally unique identifier. |
| `email` | `Nullable(String)` | Identity anchor (resolved by Identity Manager). |
| `full_name` | `Nullable(String)` | Display name. |
| `role` | `Nullable(Float64)` | Zulip role code (100 owner / 200 admin / 400 member / 600 guest). |
| `is_active` | `Nullable(Bool)` | Active-account flag. |
| `recipient_id` | `Nullable(Float64)` | Internal Zulip recipient identifier. |
| `tenant_id` | `Nullable(String)` | Stamped from `insight_tenant_id`. |
| `source_id` | `Nullable(String)` | Stamped from `insight_source_id`. |
| `unique_key` | `String` | `{tenant_id}-{source_id}-{id}`. RMT order_by. |
| `_airbyte_extracted_at` | `DateTime64(3)` | RMT `_version`. |

**Primary key**: `unique_key`.
**RMT order_by**: `(unique_key)`.
**Stream sync mode**: full refresh per run (no `incremental_sync` configured).

#### Table: `bronze_zulip_proxy.messages`

Bronze table, same engine, promoted by the same `zulip_proxy__bronze_promoted` model.

| Column | Type | Description |
|--------|------|-------------|
| `uniq` | `String` | Proxy-assigned primary key of the aggregate bucket. |
| `sender_id` | `Nullable(Float64)` | Sender's Zulip user ID; joins to `users.id`. |
| `count` | `Nullable(Float64)` | Number of messages in this bucket. |
| `created_at` | `Nullable(String)` | Bucket anchor timestamp; incremental cursor (`%Y-%m-%dT%H:%M:%S.%fZ`). |
| `tenant_id` | `Nullable(String)` | Stamped. |
| `source_id` | `Nullable(String)` | Stamped. |
| `unique_key` | `String` | `{tenant_id}-{source_id}-{uniq}`. RMT order_by. |
| `_airbyte_extracted_at` | `DateTime64(3)` | RMT `_version`. |

**Primary key**: `unique_key`.
**RMT order_by**: `(unique_key)`.
**Stream sync mode**: incremental on `created_at`.

#### Silver model: `staging.zulip_proxy__users_snapshot`

`snapshot()` macro applied to `bronze_zulip_proxy.users`, tracking the same field set as Zoom
(`first_name`-equivalent: `full_name`; status flag: `is_active`; identity field: `email`; etc.).

#### Silver model: `staging.zulip_proxy__users_fields_history`

`fields_history()` macro applied to `users_snapshot`, entity-id column `id`, tracking
`full_name`, `email`, `is_active`, `role`, `recipient_id`, `uuid`.

#### Silver model: `staging.zulip_proxy__identity_inputs`

`identity_inputs_from_history()` macro:

```
source_type='zulip_proxy'
identity_fields = [
  { field='email',     value_type='email',        value_field_name='bronze_zulip_proxy.users.email' },
  { field='full_name', value_type='display_name', value_field_name='bronze_zulip_proxy.users.full_name' },
]
deactivation_condition = "field_name = 'is_active' AND new_value = 'false'"
```

#### Silver model: `staging.zulip_proxy__collab_chat_activity`

Aggregates `bronze_zulip_proxy.messages` joined to `bronze_zulip_proxy.users` (FINAL) into
`class_collab_chat_activity` (sender's email → person_key, daily roll-up of `count`). Engine
`ReplacingMergeTree(_version)`, order_by `(unique_key)`, materialization `incremental`,
incremental_strategy `append`, tag `silver:class_collab_chat_activity`.

### 3.8 Deployment Topology

- The connector is registered as a no-code Airbyte source in the shared Insight workspace (per
  ADR-0009 — single Airbyte workspace identified by `app.kubernetes.io/part-of=insight` flag).
- Source UUID is allocated by Airbyte at first reconcile; persisted in the state file by
  `connect.sh`.
- Connection UUID writes to namespace `bronze_zulip_proxy` per the descriptor.
- Sync schedule defined in the descriptor (`schedule: "0 3 * * *"` by default — same cadence
  used by the existing `zoom` connector).
- Argo wraps the sync + dbt run in a single workflow per `run-sync.sh`. dbt selector:
  `tag:zulip_proxy+`.

## 4. Additional context

### Source Collection Strategy

`users` is collected in full on every run because the directory is small and there is no proxy-
side cursor for it. `messages` is collected incrementally on `created_at` because the bucket
volume can be unbounded.

### Incremental Strategy

`DatetimeBasedCursor` with `cursor_field=created_at`, `datetime_format='%Y-%m-%dT%H:%M:%S.%f%z'`,
`cursor_datetime_formats=['%Y-%m-%dT%H:%M:%S.%fZ']`, `start_datetime` rendered from
`config['zulip_proxy_start_date']`, `is_data_feed=true` (no end_datetime — the proxy controls the
high-water mark).

### Identity Model

Email is the only identity field this connector contributes. `id` and `uuid` are Zulip-internal
and not used by Identity Manager for cross-source resolution. The Silver
`zulip_proxy__identity_inputs` model emits per-email events into the Identity Manager pipeline.

### Pagination

- `users`: `OffsetIncrement` strategy, page size 100. Stop when a page returns fewer than 100
  records.
- `messages`: `CursorPagination` strategy, cursor field name `cursor`, response cursor at
  `response.get('nextCursor')`, stop condition `not response.get('nextCursor')`.

### Throttle

`zulip_proxy_throttle_ms` is forwarded as the `throttle` query parameter on `/api/messages`
requests only. The `users` endpoint is not throttled (small payload).

### Idempotence

`unique_key` is computed deterministically per record:

- `users`: `{tenant_id}-{source_id}-{id}`.
- `messages`: `{tenant_id}-{source_id}-{uniq}`.

RMT (`ReplacingMergeTree(_airbyte_extracted_at)`, order_by `unique_key`) collapses duplicates
across overlap windows on `OPTIMIZE` / `FINAL`.

### Run Logging and Observability

There is no connector-managed run-log table (deliberate departure from the legacy
`zulip_collection_runs` table described in the direct Zulip spec). Operational visibility is
provided by the surrounding ingestion platform:

- Argo workflow status (`./logs.sh <workflow|latest>`).
- Airbyte job logs (`./logs.sh airbyte <job-id>`).
- ClickHouse query metrics on the destination side.

If the operator wants per-run row counts, they query `bronze_zulip_proxy.users` and
`bronze_zulip_proxy.messages` grouped by `_airbyte_extracted_at`.

### Assumptions and Risks That Affect Implementation

- The proxy's `uniq` field is the authoritative primary key of aggregated message records.
  Changes to its derivation upstream would invalidate `unique_key` and produce duplicates in
  Silver. Mitigation: pinned in DESIGN.md as a contract; flagged in the reproducibility log.
- The proxy returns `created_at` in ISO-8601 with milliseconds and a trailing `Z`. If the proxy
  ever switches to a different format, the `cursor_datetime_formats` list in the manifest must be
  extended.
- The proxy does not expose stream-level schema discovery. The connector ships an
  `InlineSchemaLoader` schema derived from the reference v0.57.0 manifest; the real schema must
  be confirmed by `/connector schema` against a live proxy and the InlineSchemaLoader updated.

## 5. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **Sibling spec**: [../zulip](../../zulip/) — direct-API Basic-Auth connector covering the same
  data shape.
- **Foundation ADRs**:
  - `cpt-dataflow-adr-rmt-with-version-and-unique-key` — RMT as universal Bronze engine.
  - `cpt-dataflow-adr-promote-bronze-to-rmt` — promotion macro this connector depends on.
  - `cpt-dataflow-adr-unique-key-formula` — `{tenant}-{source}-{native_id}` formula used in both
    streams.
  - `cpt-airbyte-toolkit-adr-required-fields-in-descriptor-not-example` — drives
    `descriptor.yaml.secret.required_fields`.
  - `cpt-airbyte-toolkit-adr-airbyte-workspace-as-namespace` — connector lives in the shared
    Insight workspace.
- **Skill workflows that govern construction**:
  - `.cf-studio/.core/skills/connector/workflows/create.md` — package layout and manifest rules.
  - `.cf-studio/.core/skills/connector/workflows/validate.md` — validation checks.
- **Reproducibility log**: [../REPRODUCIBILITY-LOG.md](../REPRODUCIBILITY-LOG.md) — captures
  deviations from `create.md` (DEV-01 `start_date`, DEV-02 `url_base`, DEV-03 `throttle`).
