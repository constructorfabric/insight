# PRD — Zendesk Connector

> Version 2.0 — June 2026
> Issue: INSIGHT-459
> Status: Phase 1 + Phase 2 audit stream + Silver/Gold delivered

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
  - [4.1 In Scope (Phase 1)](#41-in-scope-phase-1)
  - [4.2 Out of Scope (Phase 1 — deferred to Phase 2)](#42-out-of-scope-phase-1--deferred-to-phase-2)
  - [4.3 Permanently Out of Scope](#43-permanently-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Ticket Data Extraction](#51-ticket-data-extraction)
  - [5.2 Agent Directory Extraction](#52-agent-directory-extraction)
  - [5.3 Satisfaction Ratings Extraction](#53-satisfaction-ratings-extraction)
  - [5.4 Connector Operations](#54-connector-operations)
  - [5.5 Data Integrity](#55-data-integrity)
  - [5.6 Identity Resolution](#56-identity-resolution)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-001 Configure Zendesk Connection](#uc-001-configure-zendesk-connection)
  - [UC-002 Incremental Sync Run](#uc-002-incremental-sync-run)
  - [UC-003 First Run / Historical Backfill](#uc-003-first-run--historical-backfill)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Resolved Questions](#13-resolved-questions)
- [14. Non-Applicable Requirements](#14-non-applicable-requirements)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

The Zendesk Connector extracts ticket data, agent directory, and customer satisfaction ratings from the Zendesk REST API v2 and loads them into the Insight platform's Bronze layer. It provides the raw material for support team performance analytics — CSAT scores, ticket volume trends, resolution rates, agent workload, and first-response/resolution time measurements.

### 1.2 Background / Problem Statement

Zendesk is the primary customer support platform used by the majority of Insight's target customers. Customer support teams are a significant portion of the workforce tracked by Insight, but they are currently invisible to the platform — no support-domain connector exists.

Support team leaders need to understand how their team is performing: How quickly do agents respond to tickets? Are SLA targets being met? Which agents or groups handle the most volume? What is the CSAT score trend? Today these questions are answered using Zendesk's own reporting, which is siloed from the engineering, sales, and HR data in Insight.

The Zendesk connector closes this gap by ingesting support activity into the same Bronze-to-Silver pipeline that serves Jira, GitHub, Slack, and other Insight sources. Once in Insight, support activity can be correlated with deployment frequency, incident data, and HR records — enabling unified workforce analytics across engineering and support.

**Target Users**:

- Platform operators who configure Zendesk API credentials and monitor extraction runs
- Data analysts who consume support activity data in Silver/Gold layers for cross-functional workforce analytics
- Support team managers who use ticket volume, resolution rate, agent workload, and CSAT data for performance analysis
- HR and workforce analytics stakeholders who need support staff activity alongside engineering metrics

**Key Problems Solved**:

- Lack of Zendesk data in Insight prevents support team inclusion in workforce analytics dashboards
- CSAT scores, first-response times, and resolution times are not available alongside engineering metrics
- Support agent identity is not resolved to canonical `person_id` — support staff cannot be linked to HR, collaboration, or other sources
- No unified view of tickets, ratings, and agents in the Insight data store

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- Zendesk ticket and agent data extracted with no missed sync windows over a 30-day period after GA (Baseline: no Zendesk extraction; Target: v1.0)
- Per-agent CSAT data available for identity resolution within 24 hours of extraction (Baseline: N/A; Target: v1.0)
- Support agent email resolution to `person_id` working end-to-end for ≥ 90% of active agents (Baseline: 0%; Target: v1.0)
- Ticket volume and CSAT trend metrics available in Insight Gold layer within 24 hours of ingestion (Baseline: N/A; Target: v1.0)

**Capabilities**:

- Extract Zendesk tickets with current state (status, priority, assignee, timing metrics) via incremental export
- Extract satisfaction ratings as a separate stream preserving full rating history
- Extract agent directory for identity resolution via `email` → `person_id`
- Incremental extraction using `updated_at` timestamp as cursor for tickets and ratings
- API token is the implemented auth method; OAuth 2.0 is spec-supported but not implemented
- Monitoring via Airbyte platform job stats / sync logs (`support_collection_runs` de-scoped)

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Zendesk REST API v2 | Zendesk's primary REST API at `https://{subdomain}.zendesk.com/api/v2/` |
| Ticket | Core support entity in Zendesk — a customer request, incident, or task. Has a lifecycle from `new` through `solved`/`closed` |
| Audit | Zendesk's per-ticket event log. Each audit is a batch of events (field changes, comments, satisfaction updates) with a single timestamp and author. Source of truth for event history. |
| Metric Set | Pre-computed timing metrics attached to tickets: `reply_time_in_minutes`, `full_resolution_time_in_minutes`, `solved_at`. Available as a side-load on the incremental ticket export. |
| Business Hours | Time measured only within the configured business schedule (excludes weekends, holidays). Zendesk SLA Policies are defined in business hours. |
| Calendar Hours | Wall-clock elapsed time regardless of business schedule. Used for cross-source comparison. |
| Satisfaction Rating | Customer's post-resolution assessment of a ticket: `good`, `bad`, or `offered` (not yet answered). |
| Agent | Zendesk user with `role = "agent"`, `"admin"`, or `"light_agent"` — an internal team member who handles tickets. |
| Requester | External customer who opened the ticket. Not an internal agent; `requester_id` is not resolved to `person_id`. |
| Group | A Zendesk group represents a team or department (e.g. Tier 1 Support, Billing). Agents are assigned to groups. |
| Bronze Table | Raw data table in ClickHouse preserving source-native field names and types without transformation. |
| Incremental Export | `GET /api/v2/incremental/tickets.json?start_time={unix_ts}` — Zendesk's bulk export endpoint, returning all tickets updated since the cursor. Preferred over full-table scans. |

---

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-zendeskspec-actor-operator`

**Role**: Configures the Zendesk connection (API credentials, subdomain, start date), deploys the K8s Secret, monitors sync runs, and manages credential rotation.

**Interaction**: configures `connector.yaml` parameters; creates and updates K8s Secret; reviews `support_collection_runs` for errors.

#### Data Analyst

**ID**: `cpt-zendeskspec-actor-analyst`

**Role**: Consumes Zendesk Bronze data via Silver/Gold models. Builds ticket volume, CSAT, and resolution time metrics. Cross-joins support data with HR and engineering sources.

**Interaction**: queries `support_tickets`, `zendesk_satisfaction_ratings`, `support_agents`, and downstream Silver/Gold tables.

#### Support Team Manager

**ID**: `cpt-zendeskspec-actor-manager`

**Role**: Consumes Gold metrics dashboards: ticket volume by team, CSAT per agent, resolution rate, and SLA compliance. Does not interact with Bronze tables directly.

### 2.2 System Actors

#### Zendesk REST API v2

**ID**: `cpt-zendeskspec-actor-zendesk-api`

**Role**: External REST API providing ticket export, satisfaction ratings, and user directory. Enforces rate limits (700 requests/minute for Business and above; lower for lower tiers). Requires authentication via API token (email/token Basic Auth) or OAuth 2.0.

#### Identity Manager

**ID**: `cpt-zendeskspec-actor-identity-manager`

**Role**: Resolves `email` from `support_agents` Bronze table to canonical `person_id` in Silver step 2. Enables cross-system joins between Zendesk support agents and other Insight sources (GitHub, Jira, Slack, HR directory, M365, etc.).

---

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires a Zendesk account with API token authentication enabled (Admin → Apps & Integrations → Zendesk API → Token Access)
- The API token must be associated with an agent or admin account with `tickets:read`, `users:read`, and `satisfaction_ratings:read` permissions
- Zendesk enforces per-minute rate limits that vary by plan tier (200 req/min on Basic; 700 req/min on Business and above). The connector MUST respect `Retry-After` headers on HTTP 429 responses
- The connector operates as a batch collector; scheduled runs should be at least daily to keep Bronze data fresh
- `support_ticket_events` (Phase 2) requires one API call per ticket — large accounts (millions of tickets) require a configurable lookback window to limit the blast radius of first-run collection
- Zendesk subdomain is the logical identifier for the connector instance (`insight_source_id = zendesk-{subdomain}`)
- Zendesk numeric IDs (ticket IDs, user IDs, group IDs) are scoped to one subdomain — multi-tenant deployments require `insight_source_id` qualification in every Bronze query

---

## 4. Scope

### 4.1 In Scope (Phase 1)

- Extraction of Zendesk tickets with current state including core fields, status, priority, assignee, group, timing metrics (both business-hours and calendar-hours variants), and full API response as JSON
- Incremental ticket extraction using Zendesk's bulk export endpoint (`GET /api/v2/incremental/tickets.json`)
- Extraction of satisfaction ratings as a separate incremental stream (`zendesk_satisfaction_ratings`)
- Extraction of the agent directory (agents and admins) for identity resolution
- Extraction of per-ticket Ticket Audits (`support_ticket_events`) from `GET /api/v2/tickets/{id}/audits`, driving actor-attributed activity (updates, public/private comments, solved-ticket counts)
- The slim key-only parent stream (`support_ticket_ids`) that fans the audit substream out (`ticket_id` + `updated_at` only) to keep the CDK parent-record cache small
- Silver `class_support_activity` person×date rollup + shared dims + Gold support metrics (`support_bullet_rows` / `support_person_period` / `support_company_stats`) + analytics Support metric sets + a frontend "Support" dashboard section
- K8s Secret-based credential management following the Insight connector secret format
- Bronze-layer table schemas for all Phase 1 streams

### 4.2 Out of Scope (Phase 1 — deferred to Phase 2)

- `zendesk_ticket_ext`: custom field key-value pairs — ticket `custom_fields[]` extraction and field metadata from `GET /api/v2/ticket_fields`
- Backfilling `support_tickets.satisfaction_score` from `zendesk_satisfaction_ratings` — Silver layer responsibility in Phase 2
- `group_name` resolution on `support_agents` via `GET /api/v2/groups` — `group_name` is NULL in Phase 1 (FR `cpt-zendeskspec-fr-group-name-resolution` is `p2`)
- `support_collection_runs`: DE-SCOPED (not deferred) — a declarative manifest cannot emit a connector-generated run-log stream; run monitoring is via Airbyte platform job stats / sync logs

### 4.3 Permanently Out of Scope

- Real-time streaming — this connector operates in batch mode
- Zendesk webhooks or event-driven collection
- Attachment downloads or embedded media extraction
- Zendesk SLA Policies table (SLA policy objects) — Zendesk pre-computes timing metrics on tickets; no explicit SLA breach object exists unlike JSM
- Zendesk macros, triggers, automations, views configuration
- Zendesk Talk (voice) or Chat (live chat) data — support ticket domain only
- Customer (`requester_id`) identity resolution to `person_id` — external customers are not in the HR roster

---

## 5. Functional Requirements

### 5.1 Ticket Data Extraction

#### Extract Tickets via Incremental Export

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-ticket-extraction`

The connector **MUST** extract Zendesk tickets using the incremental export endpoint `GET /api/v2/incremental/tickets.json?start_time={unix_ts}` with `?include=metric_sets` sideload to retrieve timing metrics in the same response. Each extracted ticket **MUST** be written to `support_tickets` with all fields defined in the Bronze schema.

**Rationale**: The incremental export endpoint is Zendesk's official bulk sync API — it uses a server-side cursor, handles pagination up to 1000 tickets per page, and is the only efficient way to sync large accounts. The `metric_sets` sideload eliminates a separate per-ticket API call for timing fields.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`

#### Store Both Business-Hours and Calendar-Hours Timing

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-timing-both-variants`

The connector **MUST** store both business-hours and calendar-hours variants of `first_reply_time` and `full_resolution_time` from `metric_set`:
- `first_reply_time_seconds` ← `metric_set.reply_time_in_minutes.business × 60`
- `first_reply_time_calendar_seconds` ← `metric_set.reply_time_in_minutes.calendar × 60`
- `full_resolution_time_seconds` ← `metric_set.full_resolution_time_in_minutes.business × 60`
- `full_resolution_time_calendar_seconds` ← `metric_set.full_resolution_time_in_minutes.calendar × 60`

All four fields **MUST** be NULL when the metric is not yet available (ticket not yet replied to / not yet resolved).

**Rationale**: SLA Policies in Zendesk are defined in business hours — business-hours values are the correct denominator for SLA compliance metrics. Calendar-hours values enable cross-source consistency with JSM (which derives timing from the event log without business-hours filtering). Storing both in Bronze avoids a re-collection when the Silver layer needs the other variant.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`, `cpt-zendeskspec-actor-analyst`

#### Store Raw Ticket Response as JSON

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-ticket-metadata`

The connector **MUST** store the full Zendesk ticket API response as a JSON string in the `metadata` field on every ticket row.

**Rationale**: The `metadata` field enables Silver/Gold queries to access fields not promoted to top-level columns without requiring a connector change or re-collection. Particularly important for future custom field support before `zendesk_ticket_ext` is implemented.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`

### 5.2 Agent Directory Extraction

#### Extract Agent Directory

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-agent-extraction`

The connector **MUST** extract all users with `role = "agent"`, `role = "admin"`, and `role = "light_agent"` from `GET /api/v2/users?role[]=agent&role[]=admin`. For each agent, the connector **MUST** store: agent ID, email, display name, role, primary group ID, and active status. (`group_name` resolution is deferred to Phase 2 — see `cpt-zendeskspec-fr-group-name-resolution`; NULL in Phase 1.)

**Rationale**: The agent directory is the identity anchor for all support analytics. Email is the cross-system key for `person_id` resolution. Group assignment enables team-level metric aggregation.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`, `cpt-zendeskspec-actor-identity-manager`

#### Resolve Group Names at Collection Time

- [ ] `p2` - **ID**: `cpt-zendeskspec-fr-group-name-resolution`

The connector **MUST** fetch group metadata via `GET /api/v2/groups` at startup and populate `group_name` by joining on `default_group_id` for each agent.

**Rationale**: Group IDs are opaque numeric identifiers. Group names enable human-readable team-level breakdowns in Silver/Gold without requiring an additional lookup table.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`

### 5.3 Satisfaction Ratings Extraction

#### Extract Satisfaction Ratings as a Separate Incremental Stream

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-ratings-extraction`

The connector **MUST** extract CSAT ratings from `GET /api/v2/satisfaction_ratings?sort_by=updated_at&sort_order=asc&start_time={unix_ts}` as a separate incremental stream, writing to `zendesk_satisfaction_ratings`. Each rating record **MUST** include: rating ID, parent ticket ID, requester ID, assignee ID, group ID, score (`good` / `bad` / `offered`), requester comment, reason label, `created_at`, and `updated_at`.

**Rationale**: Ratings arrive asynchronously — a requester may rate a ticket days after resolution. Storing ratings in a separate stream preserves the full rating history (including score changes and comments) rather than overwriting a single field on the ticket. This also means the ticket snapshot does not require re-collection when a rating arrives or changes.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`, `cpt-zendeskspec-actor-analyst`

### 5.4 Connector Operations

#### Track Collection Runs

- [ ] `p2` - **ID**: `cpt-zendeskspec-fr-collection-runs`

> **Status (v2.0): DE-SCOPED.** A declarative (nocode) Airbyte manifest cannot write a connector-generated run-log stream. Run monitoring is delegated to the Airbyte platform job stats / sync logs instead. This FR is retained for traceability but is not implemented.

Originally specified the connector to emit a `support_collection_runs` row per execution, recording: run ID (UUID), start timestamp, end timestamp, status (`running` / `completed` / `failed`), per-stream record counts, total API call count, error count, and collection settings as JSON. **DE-SCOPED**: a declarative (nocode) manifest cannot emit a connector-generated stream, so run monitoring is delegated to Airbyte platform job stats / sync logs. This paragraph is retained as historical context only and carries no normative requirement.

**Rationale**: Operational visibility into connector health. Enables alerting on failed runs and tracking data completeness over time.

**Actors**: `cpt-zendeskspec-actor-operator`

### 5.5 Data Integrity

#### Deduplicate by Primary Key

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-deduplication`

Each stream **MUST** define a primary key that ensures re-running the connector for an overlapping time window does not produce duplicate records in Bronze. Primary keys:
- `support_tickets`: `(insight_source_id, ticket_id)`
- `support_agents`: `(insight_source_id, agent_id)`
- `zendesk_satisfaction_ratings`: `(insight_source_id, rating_id)`
- `support_collection_runs`: `(run_id)`

**Rationale**: The incremental sync window may overlap with previously fetched data. `ReplacingMergeTree(_version)` storage handles deduplication at the ClickHouse level; the connector ensures the primary key uniquely identifies each record.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`

#### Inject Tenant and Instance Context on Every Record

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-tenant-context`

The connector **MUST** inject `insight_tenant_id`, `insight_source_id`, `unique_key`, `data_source`, and `collected_at` on every emitted record.

**Rationale**: Multi-tenant isolation and multi-instance disambiguation require these fields on every row. `unique_key` is the composite deduplication key used by the Airbyte destination. `data_source = "insight_zendesk"` is the discriminator for unified support domain queries across Zendesk and JSM.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`

#### Implement Incremental Sync with Cursor Persistence

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-incremental-sync`

The connector **MUST** use `DatetimeBasedCursor` on `updated_at` for `support_tickets` and `zendesk_satisfaction_ratings`. The cursor **MUST** be persisted by the Airbyte platform state mechanism and advanced only on successful page consumption. A failed run **MUST** resume from the last successful cursor position on the next attempt.

**Rationale**: Incremental sync reduces API quota usage and run duration from hours (full scan) to minutes (delta scan) for established accounts. Cursor persistence ensures no data loss on failure.

**Actors**: `cpt-zendeskspec-actor-zendesk-api`

### 5.6 Identity Resolution

#### Support Email-Based Identity Resolution

- [ ] `p1` - **ID**: `cpt-zendeskspec-fr-identity-resolution`

The connector **MUST** extract `email` from `support_agents` for every active agent. The `email` field **MUST** be stored in `support_agents` and used as the join key for downstream identity resolution to canonical `person_id` in Silver step 2.

**Rationale**: Email is the cross-system identity anchor used by the Insight Identity Manager to map Zendesk agents to the canonical person roster. Without email, support agents cannot be correlated with their activity in other Insight sources.

**Actors**: `cpt-zendeskspec-actor-identity-manager`

---

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Data Freshness

- [ ] `p1` - **ID**: `cpt-zendeskspec-nfr-freshness`

Bronze tables **MUST** reflect Zendesk state within 25 hours of the most recent scheduled run. The connector is scheduled once daily (default: 05:00 UTC). A 25-hour window provides a 1-hour buffer for run duration.

**Verification**: Monitor `support_collection_runs.completed_at` against wall clock; alert if gap exceeds 25 hours.

#### Rate Limit Compliance

- [ ] `p1` - **ID**: `cpt-zendeskspec-nfr-rate-limits`

The connector **MUST** respect Zendesk API rate limits. On HTTP 429, the connector **MUST** read the `Retry-After` header and wait the specified duration before retrying. Exponential backoff with jitter **MUST** be applied for repeated 429 responses. The connector **MUST NOT** exhaust the per-minute rate limit budget for other active integrations in the same Zendesk account.

**Verification**: Zero rate limit failures (`HTTP 429 unhandled`) in production over a 7-day window.

#### UTC Timestamps

- [ ] `p1` - **ID**: `cpt-zendeskspec-nfr-utc`

All timestamps stored in Bronze **MUST** be in UTC. Zendesk API timestamps are ISO 8601 with UTC offset; the connector **MUST** normalize to UTC before storage.

**Verification**: Schema test — zero non-UTC timestamps in Bronze.

#### Declarative Manifest Compliance

- [ ] `p1` - **ID**: `cpt-zendeskspec-nfr-declarative`

The connector **MUST** be a valid Airbyte DeclarativeSource YAML manifest. It **MUST** emit valid Airbyte Protocol messages (RECORD, STATE, LOG). Every emitted record **MUST** include `tenant_id`.

**Verification**: `validate-strict` passes via `./src/ingestion/tools/declarative-connector/source.sh validate-strict support/zendesk`.

### 6.2 NFR Exclusions

- **Latency SLA below 25h**: the connector is batch-only; sub-daily freshness is not required
- **Concurrency**: the connector runs as a single Airbyte job; no horizontal scaling requirement
- **Availability SLA**: monitored by the Airbyte platform; the connector itself has no uptime obligation

---

## 7. Public Library Interfaces

### 7.1 Public API Surface

The Zendesk connector does not expose a public library interface. It is consumed via the Airbyte protocol (RECORD, STATE, LOG messages) and orchestrated by the Airbyte platform.

**Connector entry point**: `source.sh validate-strict support/zendesk` / `source.sh run support/zendesk`

### 7.2 External Integration Contracts

| Contract | Description |
|----------|-------------|
| `support_tickets` Bronze schema | Consumed by the support domain Silver pipeline; breaking schema changes require a Silver migration |
| `support_agents` Bronze schema | Consumed by the Identity Manager in Silver step 2; `email` field is the identity contract |
| `zendesk_satisfaction_ratings` Bronze schema | Consumed by the support domain Silver pipeline for CSAT metrics |
| `insight_zendesk` data_source value | Discriminator in all unified support Bronze queries; changing this value is a breaking change |
| K8s Secret format | `metadata.name = insight-zendesk-{source_id}`; annotations `insight.cyberfabric.com/connector: zendesk`, `insight.cyberfabric.com/source-id: zendesk-{subdomain}` |

---

## 8. Use Cases

### UC-001 Configure Zendesk Connection

**Actor**: Platform Operator

**Preconditions**:
- Zendesk API token created under Admin → Apps & Integrations → Zendesk API
- Operator has the Zendesk subdomain, agent email, and API token
- Kubernetes cluster with `insight` namespace accessible

**Main Flow**:
1. Operator creates the K8s Secret (`insight-zendesk-main`) with fields: `zendesk_subdomain`, `zendesk_email`, `zendesk_api_token`, `start_date` (optional)
2. Operator applies the Secret: `kubectl apply -f zendesk-secret.yaml`
3. Reconcile loop discovers the Secret, creates an Airbyte connection with the correct parameters
4. Operator triggers a manual sync to verify connectivity
5. Connector performs a `check` connection (CheckStream against `support_agents`, i.e. the first page of `GET /api/v2/users`) — success confirms credentials are valid

**Postconditions**: Airbyte connection exists and `support_agents` has data after first sync.

**Alternative Flow — invalid credentials**: Connector returns AUTH_ERROR on check; operator is notified to update the Secret.

### UC-002 Incremental Sync Run

**Actor**: Orchestrator (automated, daily at 05:00 UTC)

**Preconditions**:
- Connection configured (UC-001 completed)
- State from previous successful run exists in Airbyte platform

**Main Flow**:
1. Orchestrator triggers sync for connection `zendesk-{subdomain}-default-conn`
2. Connector reads cursor from Airbyte state for `support_tickets` and `zendesk_satisfaction_ratings`
3. Connector fetches `GET /api/v2/incremental/tickets.json?start_time={cursor}&include=metric_sets` — paginates until end
4. Connector fetches `GET /api/v2/users?role[]=agent&role[]=admin` (full refresh, no cursor)
5. Connector fetches `GET /api/v2/satisfaction_ratings?start_time={cursor}` — paginates until end
6. Connector writes RECORD messages to Airbyte destination (ClickHouse Bronze tables)
7. Connector emits STATE message with updated cursors for tickets and ratings
8. Airbyte platform persists new state
9. Connector writes `support_collection_runs` record with `status = "completed"`

**Postconditions**: Bronze tables contain all records updated since the previous cursor. Cursor advanced to the current run's latest `updated_at`.

**Alternative Flow — rate limit hit**: Connector receives HTTP 429, reads `Retry-After`, waits, retries. Sync continues from last successful page.

### UC-003 First Run / Historical Backfill

**Actor**: Platform Operator

**Preconditions**:
- Connection configured (UC-001 completed)
- No prior Airbyte state exists (first run)

**Main Flow**:
1. Operator sets `start_date` in the K8s Secret (e.g. `2025-01-01` for a 16-month backfill)
2. Reconcile loop creates Airbyte connection with `start_date` parameter
3. Connector uses `start_date` as the initial cursor for the incremental ticket export
4. Connector paginates through all tickets updated since `start_date` at 1000 tickets/page
5. Connector fetches all agents (full refresh — no start date needed)
6. Connector fetches ratings from `start_date`
7. For large accounts, run duration may be several hours — Airbyte checkpoints state after each page

**Postconditions**: Bronze tables populated with full historical data from `start_date`. Subsequent runs are incremental.

---

## 9. Acceptance Criteria

| Criterion | Verification |
|-----------|-------------|
| Connector validates cleanly | `validate-strict` passes with zero errors |
| Tickets stream is incremental | Second run after initial sync extracts only tickets updated since the first run's cursor |
| Agents stream is full-refresh | Agent directory is fully refreshed on every run |
| Ratings stream is incremental | Only ratings updated since last cursor are fetched on subsequent runs |
| Both timing variants populated | `first_reply_time_seconds` and `first_reply_time_calendar_seconds` are non-null for solved tickets |
| `satisfaction_score` is NULL | `support_tickets.satisfaction_score` is NULL in all Phase 1 rows |
| Agent emails present | ≥ 95% of active agents in `support_agents` have non-null `email` |
| Multi-tenant isolation | Two Zendesk tenants produce rows with different `insight_source_id` values and no ID collisions |
| Rate limit handling | Connector does not abort on HTTP 429; retries after `Retry-After` |
| UTC timestamps | All `created_at`, `updated_at` fields are UTC-normalised |
| `unique_key` is unique | Zero duplicate `unique_key` values per stream per tenant |
| Audit fan-out runs without silent hang | slim key-only parent + `concurrency_level` ≥ 2 + `incremental_dependency`; live-verified |

---

## 10. Dependencies

| Dependency | Type | Notes |
|-----------|------|-------|
| Zendesk REST API v2 | External API | API token and subdomain required; rate limits apply |
| Airbyte DeclarativeSource CDK | Framework | Manifest version pinned in `connector.yaml` |
| Airbyte ClickHouse destination | Platform | Bronze table creation and upsert handled by destination |
| Identity Manager | Downstream | Silver step 2 resolves `support_agents.email` → `person_id` |
| Support domain Silver pipeline | Downstream | Consumes `support_tickets` and `zendesk_satisfaction_ratings`; Phase 2 adds `support_ticket_events` |
| `insight-toolbox` Docker image | CI/CD | `descriptor.yaml` included in image; triggers reconcile loop |

---

## 11. Assumptions

- Zendesk API token authentication is enabled on the customer's account (some Enterprise accounts enforce OAuth only)
- The customer's Zendesk plan supports the incremental export endpoint (available on all Zendesk Suite plans; may not be available on legacy Starter/Essential plans)
- Agent emails in Zendesk match the corporate email domain used by the Identity Manager (i.e., agents use their work email, not personal emails)
- Ticket IDs are stable and unique within a Zendesk subdomain — they are never reused after ticket deletion
- The customer's Zendesk account has a rate limit of at least 200 requests/minute (Basic tier minimum)

---

## 12. Risks

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Per-account rate limit exhaustion during first-run backfill for large accounts | Medium | Medium | Configurable `start_date` limits backfill window; exponential backoff on 429 |
| Agent email not available (suspended agents, privacy settings) | Low | Low | NULL email stored; agent is excluded from identity resolution but data still collected |
| Zendesk API breaking change (field names, response structure) | Low | High | Full `metadata` JSON stored on every ticket — Silver can re-derive fields without re-collection |
| Metric set not available for unresolved or very old tickets | Low | Low | All timing fields nullable; NULL used when metric not available |
| `support_ticket_events` scale in Phase 2 (millions of audits) | Medium | Medium | Phase 2 scoped to configurable lookback window; full-history backfill is operator opt-in |

---

## 13. Resolved Questions

### RQ-PRD-1: Separate stream or backfill for satisfaction ratings

**Question**: Should satisfaction ratings be backfilled directly onto `support_tickets.satisfaction_score` during collection, or stored as a separate stream?

**Decision**: Stored as a separate stream (`zendesk_satisfaction_ratings`). `support_tickets.satisfaction_score` is NULL in Phase 1; Silver layer computes the latest score per ticket from the ratings stream.

**Rationale**: A separate stream preserves full rating history (score changes, requester comments, reason codes). Backfilling onto the ticket row would silently lose intermediate scores. Rating history may be analytically useful (e.g., detecting tickets where a `bad` rating was later changed to `good`).

### RQ-PRD-2: Phase 1 stream count — why 3 streams

**Question**: Should Phase 1 include `support_ticket_events` (audit log)?

**Decision**: No. `support_ticket_events` is Phase 2. Phase 1 includes `support_tickets`, `support_agents`, and `zendesk_satisfaction_ratings`.

**Rationale**: The audit log requires one API call per ticket — a large account with 500,000 tickets needs 500,000 API calls on first run (at 700 req/min, ~12 hours). This is disproportionate for Phase 1 analytics that do not require per-event data. Phase 1 provides business value (volume trends, CSAT, agent roster) with three fast streams. Phase 2 unlocks MTTR and SLA compliance.

### RQ-PRD-3: Business-hours vs calendar-hours timing

**Decision**: Store both variants. See `cpt-zendeskspec-fr-timing-both-variants`.

---

## 14. Non-Applicable Requirements

| Requirement Category | Reason Not Applicable |
|---------------------|----------------------|
| Real-time / streaming | Connector is batch-mode only; Zendesk webhooks are out of scope |
| Attachment extraction | Support ticket content scope only; attachments are not extracted |
| Multi-region deployment | Handled by Insight platform infrastructure; not connector-level |
| GDPR data erasure | Handled by platform-level data retention policies; not connector-level |
| SLA Policy object extraction | Zendesk pre-computes timing on tickets; no explicit SLA policy breach objects via API (unlike JSM) |
