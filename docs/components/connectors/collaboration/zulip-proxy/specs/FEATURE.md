# Feature: Zulip-Proxy Connector — Bronze + Silver Bring-up


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Operator Configures and Runs First Sync](#operator-configures-and-runs-first-sync)
  - [Scheduled Incremental Sync of `messages`](#scheduled-incremental-sync-of-messages)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Render Manifest Config From K8s Secret](#render-manifest-config-from-k8s-secret)
  - [Bronze→Silver Promotion and Identity Inputs](#bronzesilver-promotion-and-identity-inputs)
- [4. States (CDSL)](#4-states-cdsl)
  - [Messages Cursor State](#messages-cursor-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Package Files Present](#package-files-present)
  - [dbt Models Present](#dbt-models-present)
  - [Secret Example Present](#secret-example-present)
  - [Artifacts Registered](#artifacts-registered)
  - [All Validators Pass](#all-validators-pass)
  - [Live Smoke Tests Pass](#live-smoke-tests-pass)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Traceability](#7-traceability)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-insightspec-featstatus-zulip-proxy-bringup`
## 1. Feature Context

### 1.1 Overview

Bring the Zulip-Proxy connector live end-to-end: package + manifest, K8s Secret discovery, dbt
Silver models, identity inputs. Result: a tenant with a configured proxy and Bearer token sees
fresh `bronze_zulip_proxy.users` and `bronze_zulip_proxy.messages`, and Identity Manager picks up
the new emails on the next Silver run.

### 1.2 Purpose

Satisfies the PRD requirements for collecting the Zulip user directory and aggregated message
counts through the proxy, and the DESIGN principles that govern transport, schema, and identity
contribution.

**Requirements**:
`cpt-insightspec-fr-zulip-proxy-user-directory`,
`cpt-insightspec-fr-zulip-proxy-user-stamping`,
`cpt-insightspec-fr-zulip-proxy-message-incremental`,
`cpt-insightspec-fr-zulip-proxy-backfill-start-date`,
`cpt-insightspec-fr-zulip-proxy-throttle-hint`,
`cpt-insightspec-fr-zulip-proxy-message-stamping`,
`cpt-insightspec-fr-zulip-proxy-no-content`,
`cpt-insightspec-fr-zulip-proxy-idempotence`,
`cpt-insightspec-fr-zulip-proxy-401-surfaces`,
`cpt-insightspec-fr-zulip-proxy-transient-resilience`,
`cpt-insightspec-nfr-zulip-proxy-freshness`,
`cpt-insightspec-nfr-zulip-proxy-credential-blast-radius`,
`cpt-insightspec-nfr-zulip-proxy-state-idempotence`

**Principles**:
`cpt-insightspec-principle-zulip-proxy-manifest-driven`,
`cpt-insightspec-principle-zulip-proxy-config-transport`,
`cpt-insightspec-principle-zulip-proxy-no-content`

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-insightspec-actor-zulip-proxy-operator` | Mints Bearer token, applies the K8s Secret, runs the first manual sync, monitors subsequent scheduled syncs. |
| `cpt-insightspec-actor-zulip-proxy-source` | Serves `/api/users` and `/api/messages`; honors `throttle` and `Retry-After`. |
| `cpt-insightspec-actor-zulip-proxy-bronze-ingestion` | Persists Bronze rows, runs dbt models, exposes ClickHouse-side observability. |
| `cpt-insightspec-actor-zulip-proxy-analyst` | Consumes Silver/Gold downstream once Bronze is populated. |

### 1.4 References

- **PRD**: [PRD.md](./PRD.md)
- **Design**: [DESIGN.md](./DESIGN.md)
- **Reproducibility log**: [../REPRODUCIBILITY-LOG.md](../REPRODUCIBILITY-LOG.md)
- **Skill workflows**: `.cf-studio/.core/skills/connector/workflows/create.md`,
  `.cf-studio/.core/skills/connector/workflows/test.md`,
  `.cf-studio/.core/skills/connector/workflows/validate.md`
- **Reference manifest** (incompatible 0.57.0): `zulip_proxy.yaml` (local workspace copy, not in repo)
- **Closest existing connector for dbt patterns**: `src/ingestion/connectors/collaboration/zoom/`
- **Dependencies**: `promote_bronze_to_rmt`, `identity_inputs_from_history`, `snapshot`,
  `fields_history` macros (already exist in `src/ingestion/dbt/macros/`).

## 2. Actor Flows (CDSL)

User-facing interactions for this feature.

**Use cases**: `cpt-insightspec-usecase-zulip-proxy-refresh-users`,
`cpt-insightspec-usecase-zulip-proxy-collect-messages`.

### Operator Configures and Runs First Sync

- [ ] `p1` - **ID**: `cpt-insightspec-flow-zulip-proxy-bringup-first-sync`

**Actor**: `cpt-insightspec-actor-zulip-proxy-operator`

**Success Scenarios**:
- The proxy is reachable, the Bearer token is valid, and both streams complete a full pass with
  no errors. Bronze tables exist and are populated.

**Error Scenarios**:
- The Bearer token is invalid → run fails with a 401 message containing connector name and
  source-id.
- The proxy URL is unreachable from the cluster → run fails with a connection error in the Argo
  log; operator fixes networking and retries.

**Steps**:
1. [ ] - `p1` - Operator copies `src/ingestion/secrets/connectors/zulip-proxy.yaml.example` to
   `src/ingestion/secrets/connectors/zulip-proxy.yaml`, fills in real values, applies with
   `kubectl apply -f`. - `inst-zp-first-sync-1`
2. [ ] - `p1` - API: GET `/api/v1/sources/check_connection` via `connect.sh` triggers
   `CheckStream` against the proxy's `/api/users`. - `inst-zp-first-sync-2`
3. [ ] - `p1` - **IF** check passes - `inst-zp-first-sync-3`
   1. [ ] - `p1` - Operator runs `./run-sync.sh zulip-proxy <tenant>` to submit an Argo
      workflow. - `inst-zp-first-sync-3a`
4. [ ] - `p1` - **ELSE** the operator inspects the response (401 / 5xx / DNS) and corrects the
   Secret or networking. - `inst-zp-first-sync-4`
5. [ ] - `p1` - API: Airbyte sync step pulls `users` then `messages`; emits RECORD + STATE per
   stream. - `inst-zp-first-sync-5`
6. [ ] - `p1` - DB: APPEND into `bronze_zulip_proxy.users` and `bronze_zulip_proxy.messages` via
   the shared ClickHouse destination. - `inst-zp-first-sync-6`
7. [ ] - `p1` - dbt step runs `zulip_proxy__bronze_promoted` first (promotes both bronze tables
   to RMT), then snapshot → fields_history → identity_inputs → class_collab_chat_activity.
   - `inst-zp-first-sync-7`
8. [ ] - `p1` - **RETURN** Argo workflow succeeded; operator confirms row counts via
   ClickHouse. - `inst-zp-first-sync-8`

### Scheduled Incremental Sync of `messages`

- [ ] `p1` - **ID**: `cpt-insightspec-flow-zulip-proxy-bringup-incremental-sync`

**Actor**: `cpt-insightspec-actor-zulip-proxy-source` (timer-triggered via Argo schedule)

**Success Scenarios**:
- A scheduled run picks up only the new `messages` aggregates since the last `created_at`; the
  `users` stream is fully refreshed; resume reads return zero new rows when run twice in a row.

**Error Scenarios**:
- The proxy returns 401 → run fails; operator notified.
- The proxy returns 429 with `Retry-After` → connector backs off; if the retry window is
  exhausted, the run fails and is rescheduled.

**Steps**:
1. [ ] - `p1` - Argo timer fires per the descriptor's `schedule`. - `inst-zp-inc-1`
2. [ ] - `p1` - Airbyte sync reads the cursor state (`messages.created_at`) and renders
   `start_datetime` accordingly. - `inst-zp-inc-2`
3. [ ] - `p1` - API: GET `/api/users` (full refresh, paginated by `limit`/`offset`). -
   `inst-zp-inc-3`
4. [ ] - `p1` - API: GET `/api/messages?throttle={ms}&cursor={state}` (incremental, cursor
   pagination). - `inst-zp-inc-4`
5. [ ] - `p1` - DB: APPEND rows; emit STATE with the new high-water `created_at`. -
   `inst-zp-inc-5`
6. [ ] - `p1` - dbt step runs; RMT collapses duplicates in `messages` on next OPTIMIZE/FINAL. -
   `inst-zp-inc-6`
7. [ ] - `p1` - **RETURN** Argo workflow succeeded; Silver tables include only the new rows. -
   `inst-zp-inc-7`

## 3. Processes / Business Logic (CDSL)

Internal pieces that support the actor flows.

### Render Manifest Config From K8s Secret

- [ ] `p2` - **ID**: `cpt-insightspec-algo-zulip-proxy-bringup-render-config`

**Input**: K8s Secret labeled `app.kubernetes.io/part-of: insight` annotated
`insight.cyberfabric.com/connector: zulip-proxy` and `insight.cyberfabric.com/source-id:
{instance-id}`.

**Output**: An Airbyte source configuration object whose fields map 1:1 to
`spec.connection_specification` in the manifest.

**Steps**:
1. [ ] - `p1` - `connect.sh` lists Secrets by label and filters by the `connector` annotation. -
   `inst-zp-render-1`
2. [ ] - `p1` - For each Secret, read `stringData` and merge with `insight_tenant_id` (from
   tenant YAML) and `insight_source_id` (from the `source-id` annotation). - `inst-zp-render-2`
3. [ ] - `p1` - **IF** any of `zulip_proxy_base_url`, `zulip_proxy_api_key`,
   `zulip_proxy_start_date` is missing - `inst-zp-render-3`
   1. [ ] - `p1` - Fail with a clear message naming the missing field and the Secret name. -
      `inst-zp-render-3a`
4. [ ] - `p1` - **ELSE** call `POST /api/v1/sources/update` with the merged config. -
   `inst-zp-render-4`
5. [ ] - `p1` - **RETURN** the updated Airbyte source UUID. - `inst-zp-render-5`

### Bronze→Silver Promotion and Identity Inputs

- [ ] `p2` - **ID**: `cpt-insightspec-algo-zulip-proxy-bringup-promote-and-identity`

**Input**: Bronze tables `bronze_zulip_proxy.users` and `bronze_zulip_proxy.messages` (just
APPENDED by Airbyte).

**Output**: Silver models populated; `identity_inputs` enriched with new Zulip emails.

**Steps**:
1. [ ] - `p1` - dbt executes `zulip_proxy__bronze_promoted` first (model with `depends_on: false`
   ordering implicit via the `-- depends_on:` header in downstream models). - `inst-zp-promote-1`
2. [ ] - `p1` - The macro `promote_bronze_to_rmt` runs once per bronze table; subsequent calls
   are no-ops. - `inst-zp-promote-2`
3. [ ] - `p1` - `zulip_proxy__users_snapshot` runs as `incremental append` to capture user-state
   changes. - `inst-zp-promote-3`
4. [ ] - `p1` - `zulip_proxy__users_fields_history` materializes the SCD2 history. -
   `inst-zp-promote-4`
5. [ ] - `p1` - `zulip_proxy__identity_inputs` emits one row per identity field change into the
   shared `identity_inputs` table. - `inst-zp-promote-5`
6. [ ] - `p1` - `zulip_proxy__collab_chat_activity` joins `messages` to `users` (FINAL) on
   `sender_id = id` and aggregates per `(tenant, source, lower(email), date)`. -
   `inst-zp-promote-6`
7. [ ] - `p1` - **RETURN** Silver run succeeded; Identity Manager sees new identity inputs on the
   next refresh. - `inst-zp-promote-7`

## 4. States (CDSL)

States the connector / surrounding ingestion platform reasons about during a run.

### Messages Cursor State

- [ ] `p1` - **ID**: `cpt-insightspec-state-zulip-proxy-messages-cursor`

The only persistent state owned by this connector is the `messages` stream cursor — the largest
`created_at` value seen so far. It is managed by the Airbyte CDK runtime and serialized into the
job's STATE messages; the connector itself only reads it via `DatetimeBasedCursor` and emits
updated values after each page.

**States**:
- `UNINITIALIZED` — no STATE has been persisted yet. On entry, the connector uses
  `zulip_proxy_start_date` as the lower bound.
- `ACTIVE` — STATE persisted with a `created_at` value strictly greater than
  `zulip_proxy_start_date`. On entry, the connector resumes from `STATE.created_at`.
- `STALLED` — the resume read returned zero records AND `nextCursor` was null. Cursor stays at
  the last `ACTIVE` value; no movement is the expected behavior between syncs.

**Transitions**:
- `UNINITIALIZED` → `ACTIVE`: first sync emits at least one record; STATE is written.
- `ACTIVE` → `ACTIVE`: subsequent sync emits ≥1 new record; STATE advances.
- `ACTIVE` → `STALLED`: sync emits zero records; STATE unchanged.
- `STALLED` → `ACTIVE`: a later sync sees new records; STATE advances.

There is no reset transition controlled by the connector. Resetting the cursor (re-backfill from
`zulip_proxy_start_date`) is an operator action via `/connector reset zulip-proxy <tenant>`.

## 5. Definitions of Done

Conditions that must all hold true before this feature is considered complete.

### Package Files Present

- [ ] `p1` - **ID**: `cpt-insightspec-dod-zulip-proxy-bringup-package-files`

`connector.yaml` (v7.0.4), `descriptor.yaml`, `credentials.yaml.example`,
`configured_catalog.json`, and `README.md` are present in
`src/ingestion/connectors/collaboration/zulip-proxy/`.

### dbt Models Present

- [ ] `p1` - **ID**: `cpt-insightspec-dod-zulip-proxy-bringup-dbt-models`

The following dbt models exist under `src/ingestion/connectors/collaboration/zulip-proxy/dbt/`:
`zulip_proxy__bronze_promoted`, `zulip_proxy__users_snapshot`,
`zulip_proxy__users_fields_history`, `zulip_proxy__identity_inputs`,
`zulip_proxy__collab_chat_activity`. `schema.yml` declares the `bronze_zulip_proxy` source and
model tests.

### Secret Example Present

- [ ] `p1` - **ID**: `cpt-insightspec-dod-zulip-proxy-bringup-secret-example`

`src/ingestion/secrets/connectors/zulip-proxy.yaml.example` exists with all documented fields
(`zulip_proxy_base_url`, `zulip_proxy_api_key`, `zulip_proxy_start_date`,
`zulip_proxy_throttle_ms`) and is committed.

### Artifacts Registered

- [ ] `p1` - **ID**: `cpt-insightspec-dod-zulip-proxy-bringup-artifacts-registered`

PRD, DESIGN, and FEATURE are registered in `.cf-studio/config/artifacts.toml` under the
`insightspec` system.

### All Validators Pass

- [ ] `p1` - **ID**: `cpt-insightspec-dod-zulip-proxy-bringup-validators-pass`

`cfs validate` PASS for PRD, DESIGN, FEATURE. `/connector validate zulip-proxy` PASS. Both
`validate-strict` and `validate` PASS against the manifest. `/check-dbt-conventions` PASS.

### Live Smoke Tests Pass

- [ ] `p1` - **ID**: `cpt-insightspec-dod-zulip-proxy-bringup-live-smoke`

Live `check`/`discover`/per-stream `read` PASS against the operator-provided proxy instance with
a real K8s Secret applied; first Argo sync completes; Bronze tables populated; dbt models
materialize without errors; Identity Manager picks up Zulip emails from
`zulip_proxy__identity_inputs`.

## 6. Acceptance Criteria

These restate the acceptance criteria from the PRD §9 in feature terms so they can be checked
off as the bring-up progresses:

- [ ] On a fresh tenant, the connector populates `bronze_zulip_proxy.users` and
  `bronze_zulip_proxy.messages` end-to-end with `tenant_id`, `source_id`, `unique_key` on every
  row.
- [ ] A repeated sync with an unchanged proxy produces zero new Silver rows after RMT dedup.
- [ ] A 401 response from the proxy fails the run with an operator-actionable log line that
  names `connector=zulip-proxy` and the offending `source_id`.
- [ ] The Insight ingestion cluster holds no Zulip primary credentials anywhere on disk — only
  the Bearer token (in the K8s Secret) and the proxy URL.
- [ ] `/check-dbt-conventions` passes for the connector's dbt models (engine, order_by,
  append-only sync, RMT promotion).
- [ ] `cfs validate` passes for the PRD, DESIGN, and FEATURE artifacts.
- [ ] First-read on `messages` from empty state returns >0 records; resume-read from the
  persisted STATE returns a strict subset (usually 0) — proving the cursor advances.

## 7. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **DESIGN**: [DESIGN.md](./DESIGN.md)
- **Reproducibility log**: [../REPRODUCIBILITY-LOG.md](../REPRODUCIBILITY-LOG.md)
- **Related ADRs**:
  - `cpt-dataflow-adr-promote-bronze-to-rmt`
  - `cpt-dataflow-adr-rmt-with-version-and-unique-key`
  - `cpt-dataflow-adr-unique-key-formula`
  - `cpt-airbyte-toolkit-adr-required-fields-in-descriptor-not-example`
