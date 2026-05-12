# PRD — Cursor Connector


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
  - [5.1 Team Data Extraction](#51-team-data-extraction)
  - [5.2 Usage & Activity Extraction](#52-usage--activity-extraction)
  - [5.3 Connector Operations](#53-connector-operations)
  - [5.4 Data Integrity](#54-data-integrity)
  - [5.5 Identity Resolution](#55-identity-resolution)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Configure Cursor Connection](#configure-cursor-connection)
  - [Incremental Sync Run](#incremental-sync-run)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Open Questions](#13-open-questions)
- [14. Non-Applicable Requirements](#14-non-applicable-requirements)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Cursor Connector extracts team membership, audit logs, individual AI usage events, and daily aggregated usage data from the Cursor Admin API and loads them into the Insight platform's Bronze layer. It covers four data areas — Team Directory, Audit Logs, Usage Events, and Daily Usage — providing visibility into how an organization uses the Cursor AI IDE for software development.

### 1.2 Background / Problem Statement

Cursor is an AI-first code editor used by development teams for AI-assisted coding (chat, inline edit, composer, agent mode, tab completions). Understanding how teams use Cursor is essential for measuring AI adoption intensity, tracking cost per user, and comparing AI dev tool usage patterns across Cursor, Windsurf, and GitHub Copilot.

The Cursor Admin API provides four endpoint groups:
The Cursor Admin API provides four data areas:

- **Team members** — team member directory
- **Audit logs** — security and administrative events
- **Usage events** — individual AI invocation events with model, cost, and optional token breakdown
- **Daily usage** — daily per-user aggregated activity metrics (chat, composer, agent, tab completions, lines added/deleted)

The API uses Basic authentication with a team API key. Usage and daily-usage data require date-range parameters; audit logs use time-range parameters.

**Target Users**:

- Platform operators who configure the Cursor API key and monitor extraction runs
- Data analysts who consume Cursor usage data in Silver/Gold layers alongside Windsurf and GitHub Copilot
- Engineering managers who use AI dev tool metrics for adoption tracking and cost management

**Key Problems Solved**:

- Continuous extraction of Cursor usage data into the Insight Bronze layer
- Per-user AI invocation events with model, cost, and token breakdown for cost analytics
- Daily aggregated metrics (chat, agent, composer, tab completions, lines added) for productivity analysis
- Team membership and audit logs for security and compliance visibility
- Identity-resolved data via `email`/`userEmail`, enabling joins with other source systems

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- Usage event data extracted continuously with no missed billing periods (Baseline: no extraction; Target: Q2 2026) 
- Per-user daily usage records available for identity resolution within 24 hours of collection (Baseline: N/A; Target: Q2 2026)
- Cursor data integrated with Windsurf and GitHub Copilot in the unified AI dev tool analytics layer (Baseline: Cursor only; Target: Q3 2026)

**Capabilities**:

- Extract team member directory from `GET /teams/members` for identity resolution
- Extract audit log events from `GET /teams/audit-logs` for security visibility
- Extract individual AI usage events from `POST /teams/filtered-usage-events` with cost and token data
- Extract daily aggregated usage from `POST /teams/daily-usage-data` with per-user activity metrics
- Incremental extraction using timestamp/date-based cursors to avoid re-fetching
- Identity resolution via `email` (from members and daily usage) and `userEmail` (from events and audit logs)

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Cursor Admin API | Cursor's REST API for team administration. Endpoints under `https://api.cursor.com/teams/` provide team membership, audit logs, usage events, and daily usage data. |
| Basic Authentication | Cursor's authentication method. The API key is sent as `Basic base64(api_key + ':')` in the Authorization header. |
| Usage Event | A single AI invocation in Cursor — one chat message, one tab completion, one agent step, etc. Each event records the model used, cost, and optional token breakdown. |
| Daily Usage | One row per user per date, aggregating all AI activity (chat requests, composer requests, agent requests, tab completions, lines added/deleted, etc.). |
| Billing Period | Cursor billing cycles start on the 27th of each month at 12:08:04 UTC. Usage events may be retroactively adjusted within the current billing period. |
| Daily Data Cutoff | 12:08:04 UTC — the boundary at which Cursor finalizes the previous day's usage data. The daily resync stream (`cursor_usage_events_daily_resync`) uses this time as its window boundary: yesterday 12:08:04 UTC → today 12:08:04 UTC. The `connector.yaml` encodes this in `start_datetime`/`end_datetime` of the resync stream's `incremental_sync` section. |
| `tokenUsage` | Nested JSON object on usage events containing `inputTokens`, `outputTokens`, `cacheReadTokens`, `cacheWriteTokens`, `totalCents`. May be `null` when token detail is unavailable. |
| Bronze Table | Raw data table in the destination, preserving source-native field names and types without transformation. |
| `data_source` | Discriminator field set to `insight_cursor` in all Bronze rows, enabling multi-source queries in Silver/Gold. |

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-insightspec-actor-cursor-operator`

**Role**: Obtains the Cursor team API key from the Cursor dashboard, provides it to the connector, and monitors extraction runs.
**Needs**: Ability to configure the connector with the API key and verify that data is flowing correctly for all four streams.

#### Data Analyst

**ID**: `cpt-insightspec-actor-cursor-analyst`

**Role**: Consumes Cursor usage data from Silver/Gold layers to build dashboards combining AI dev tool metrics across Cursor, Windsurf, and GitHub Copilot.
**Needs**: Complete, gap-free usage data with identity resolution to canonical person IDs for cross-platform aggregation.

### 2.2 System Actors

#### Cursor Admin API

**ID**: `cpt-insightspec-actor-cursor-api`

**Role**: External REST API providing team membership, audit logs, usage events, and daily usage data. Enforces rate limits (HTTP 429) and requires Basic authentication with team API key.

#### Identity Manager

**Ref**: `cpt-insightspec-actor-identity-manager`

Resolves `email`/`userEmail` from Cursor Bronze tables to canonical `person_id` in Silver step 2. Enables cross-system joins (Cursor + Windsurf + GitHub Copilot + GitHub + Jira, etc.).

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires a Cursor team account with Admin API access and a valid API key
- The team must be on a Business or Enterprise plan (API availability may vary by plan)
- Authentication uses Basic auth with the team API key
- The connector **SHOULD** run hourly for usage events (to capture near-real-time AI activity) and daily for daily usage and audit logs
- Cursor API enforces rate limits; the connector must handle HTTP 429 responses with retry and backoff
- Usage events within the current billing period (starting 27th of month) may be retroactively adjusted — the connector should re-fetch the full billing period on each sync

## 4. Scope

### 4.1 In Scope

- Connector execution monitoring via a collection runs stream
- Identity resolution via `email` and `userEmail`
- Bronze-layer table schemas for all streams

### 4.2 Out of Scope

- Gold layer transformations and cross-source aggregation — responsibility of the AI dev tool domain pipeline
- Silver step 2 (identity resolution: `email` → `person_id`) — responsibility of the Identity Manager
- Usage events real-time streaming — the connector operates in batch mode (hourly + daily)
- Cursor workspace or project-level analytics (not available in current API)
- Real-time streaming — this connector operates in batch mode

## 5. Functional Requirements

### 5.1 Team Data Extraction

#### Extract Team Members

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cursor-team-members`

The connector **MUST** extract the team member directory, including member identity attributes (name, email), role, and removal status.

**Rationale**: The team directory provides the `email` identity key for cross-system resolution and the membership roster for understanding team composition and license utilisation.

**Actors**: `cpt-insightspec-actor-cursor-api`, `cpt-insightspec-actor-cursor-analyst`

#### Extract Audit Logs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cursor-audit-logs`

The connector **MUST** extract audit log events from the `GET /teams/audit-logs` endpoint, including: event ID, timestamp, user email, event type, event data (JSON), and IP address.

**Rationale**: Audit logs provide security and compliance visibility into team administration actions (member additions/removals, role changes, settings modifications).

**Actors**: `cpt-insightspec-actor-cursor-api`, `cpt-insightspec-actor-cursor-operator`

### 5.2 Usage & Activity Extraction

#### Extract Usage Events

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cursor-usage-events`

The connector **MUST** extract individual AI usage events from the `POST /teams/filtered-usage-events` endpoint, including: timestamp, user email, event kind (`chat`, `completion`, `agent`, `cmd-k`, etc.), model, cost (`requestsCosts`), token fee (`cursorTokenFee`), billing flags (`isTokenBasedCall`, `isFreeBugbot`), and the nested `tokenUsage` object (when present).

**Rationale**: Usage events are the most granular Cursor signal, enabling per-model cost analysis, per-user AI adoption tracking, and detailed usage pattern analytics. The nested `tokenUsage` object (containing `inputTokens`, `outputTokens`, `cacheReadTokens`, `cacheWriteTokens`, `totalCents`) is preserved as-is at Bronze level for Silver layer processing.

**Actors**: `cpt-insightspec-actor-cursor-api`, `cpt-insightspec-actor-cursor-analyst`

#### Extract Daily Usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cursor-daily-usage`

The connector **MUST** extract daily aggregated usage data from the `POST /teams/daily-usage-data` endpoint, including: user ID, email, date, activity flag (`isActive`), request counts (chat, composer, agent, cmd-k, bugbot), tab completion metrics (shown, accepted), code change metrics (lines added/deleted, accepted lines), model and extension metadata, client version, and billing breakdown (subscription, usage-based, API key requests).

**Rationale**: Daily usage provides the aggregated view of AI adoption per user per day, feeding cross-platform comparison with Windsurf and GitHub Copilot. This is the primary data source for adoption dashboards and productivity metrics.

**Actors**: `cpt-insightspec-actor-cursor-api`, `cpt-insightspec-actor-cursor-analyst`

### 5.3 Connector Operations

#### Track Collection Runs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cursor-collection-runs`

#### Capture Retroactive Cost Adjustments

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cursor-dual-sync`

The connector **MUST** ensure that usage event cost fields reflect the latest values from the Cursor API, accounting for retroactive adjustments that may occur within the current billing period.

**Rationale**: Cursor may retroactively adjust cost fields for events within the billing period. The connector must have a mechanism to capture these adjustments so that downstream cost reporting remains accurate.

**Implementation note**: See DESIGN.md for the dual-schedule sync pattern (hourly incremental + daily resync).

**Actors**: `cpt-insightspec-actor-cursor-operator`

**Actors**: `cpt-insightspec-actor-cursor-operator`

### 5.4 Data Integrity

#### Deduplicate by Primary Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cursor-deduplication`

Each stream **MUST** use a primary key to ensure that re-running the connector for an overlapping date range does not produce duplicate records:

- `cursor_members`: key = `email`
- `cursor_audit_logs`: key = `event_id`
- `cursor_usage_events`: key = `unique` (computed as `userEmail + timestamp`)
- `cursor_usage_events_daily_resync`: key = `unique` (same schema as `cursor_usage_events`)
- `cursor_daily_usage`: key = `unique` (computed as `email + date`)

**Rationale**: The incremental sync window may overlap with previously fetched dates. Usage events within the billing period are re-fetched on each sync to capture retroactive adjustments. Deduplication ensures idempotent extraction.

**Actors**: `cpt-insightspec-actor-cursor-api`

### 5.5 Identity Resolution

#### Expose Identity Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cursor-identity-key`

All data streams **MUST** include `email` or `userEmail` as a non-null identity field. This field is used by the Identity Manager to resolve Cursor users to canonical `person_id` values in the Silver layer.

**Exemption**: The monitoring stream `cursor_collection_runs` does not carry a user identity field — it records connector execution metadata and is excluded from the identity resolution requirement.

**Rationale**: Cross-system identity resolution is the foundation of the Insight platform's analytics. Email is a reliable, stable identifier shared across Cursor, Windsurf, GitHub Copilot, and most enterprise systems.

**Actors**: `cpt-insightspec-actor-identity-manager`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Data Freshness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cursor-freshness`

The connector **MUST** deliver usage event data to the Bronze layer within 1 hour of the connector's scheduled run (for hourly sync) or within 24 hours (for daily sync).

**Threshold**: Data available in Bronze ≤ 1h (events) / ≤ 24h (daily usage, audit logs, members) after scheduled collection time.

**Rationale**: Near-real-time AI usage visibility enables timely cost monitoring and adoption tracking. Daily usage and audit logs are less time-sensitive.

#### Extraction Completeness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cursor-completeness`

The connector **MUST** extract 100% of records reported by the Cursor API for each stream on each run, with zero record loss.

**Threshold**: Records extracted = records available in API (per stream, per date range).

**Rationale**: Partial extraction leads to incorrect per-user metrics, understated costs, and unreliable adoption comparisons across AI dev tools.

#### Cost Accuracy

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cursor-cost-accuracy`

Usage event cost fields (`requestsCosts`, `cursorTokenFee`, `totalCents`) **MUST** reflect the latest values from the API after the daily resync covers the full billing period.

**Threshold**: Cost values in Bronze match API values within 24 hours of any retroactive adjustment.

**Rationale**: Cursor may retroactively adjust costs within the billing period. Stale cost data leads to incorrect budget reporting.

### 6.2 NFR Exclusions

- **Throughput / latency**: Not applicable for daily usage and members (low volume). Usage events may be high-volume (thousands per day for large teams) but the API handles pagination natively.
- **Availability**: Batch connector — availability is determined by the orchestrator's scheduling, not by this connector.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Cursor Stream Contract

- [ ] `p1` - **ID**: `cpt-insightspec-interface-cursor-streams`

**Type**: Data format (Bronze table schemas)

**Stability**: stable

**Description**: Four Bronze streams with defined schemas — `cursor_members`, `cursor_audit_logs`, `cursor_usage_events`, `cursor_daily_usage`. Usage streams use `email`/`userEmail` as the identity key and `timestamp`/`date` as cursor fields.

**Breaking Change Policy**: Adding new fields is non-breaking. Removing or renaming fields requires a migration.

### 7.2 External Integration Contracts

#### Cursor Admin API

- [ ] `p1` - **ID**: `cpt-insightspec-contract-cursor-admin-api`

**Direction**: required from external system

**Protocol/Format**: REST / JSON

| Stream | Endpoint | Method |
|--------|----------|--------|
| `cursor_members` | `GET /teams/members` | Full refresh |
| `cursor_audit_logs` | `GET /teams/audit-logs?startTime=&endTime=&page=&pageSize=` | Incremental |
| `cursor_usage_events` | `POST /teams/filtered-usage-events` (body: `{startDate, endDate, page, pageSize}`) | Incremental |
| `cursor_daily_usage` | `POST /teams/daily-usage-data` (body: `{startDate, endDate, page, pageSize}`) | Incremental |

**Authentication**: Basic auth — `Authorization: Basic base64(api_key + ':')`

**Compatibility**: Cursor Admin API. Response format is JSON with endpoint-specific pagination. Field additions are non-breaking.

## 8. Use Cases

### Configure Cursor Connection

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-cursor-configure`

**Actor**: `cpt-insightspec-actor-cursor-operator`

**Preconditions**:

- Cursor team account with Admin API access
- Team API key generated from Cursor dashboard

**Main Flow**:

1. Operator provides the Cursor team API key
2. System validates credentials against the Cursor API (`GET /teams/members`)
3. System initializes the connection with empty state

**Postconditions**:

- Connection is ready for first sync run

**Alternative Flows**:

- **Invalid API key**: System reports authentication failure (HTTP 401); operator corrects API key
- **Insufficient plan**: API returns 403; system reports that a Business/Enterprise plan may be required
- **Rate limited on check**: System retries after 60s; reports success if retry succeeds

### Incremental Sync Run

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-cursor-incremental-sync`

**Actor**: `cpt-insightspec-actor-cursor-operator`

**Preconditions**:

- Connection configured and credentials valid
- Previous state available (or empty for first run)

**Main Flow**:

1. Orchestrator triggers the connector with current state
2. Connector fetches team members from `GET /teams/members` (full refresh)
3. Connector fetches audit logs from `GET /teams/audit-logs` for the time range from last cursor to now
4. Connector fetches usage events from `POST /teams/filtered-usage-events` for the date range from last cursor to now
5. Connector fetches daily usage from `POST /teams/daily-usage-data` for the date range from last cursor to now
6. For each stream: paginate through all pages, emit records with deduplication keys
7. Updated cursor positions captured after successful write

**Postconditions**:

- Bronze tables contain new records
- State updated with latest cursor positions for each stream

**Alternative Flows**:

- **First run (members)**: Full refresh — all current team members extracted
- **First run (audit logs, events, daily usage)**: Connector extracts data for the last 30 days (configurable lookback)
- **API throttling (HTTP 429)**: Connector retries with backoff (60s delay)
- **Empty date range**: No new data to fetch; sync completes with zero records

## 9. Acceptance Criteria

- Team members stream extracts all members from a live Cursor team account
- Audit log stream extracts events within the date range, including event type and user email
- Usage events stream extracts individual AI invocations with model, cost, kind, and optional `tokenUsage` nested object
- Daily usage stream extracts per-user daily aggregates with all activity metrics (chat, composer, agent, tabs, lines)
- Incremental sync on second run extracts only new data (no duplicates for members and audit logs; upsert for events and daily usage)
- `email`/`userEmail` is present in every record across all data streams except the monitoring stream (`cursor_collection_runs`), which does not carry user identity fields
- Pagination is exhausted for all paginated endpoints (no truncated results)
- Basic authentication works correctly with the team API key
- `tenant_id` is present in every record emitted by the connector (injected via `AddFields` transformation in the manifest `spec.connection_specification`)

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Cursor Admin API | Team, audit, usage, and daily usage endpoints | `p1` |
| Cursor team API key | Authentication credential | `p1` |
| Airbyte Declarative Connector framework (latest) | Execution model for running the connector | `p1` |
| Identity Manager | Resolves `email`/`userEmail` to `person_id` in Silver | `p2` |
| Destination store (PostgreSQL / ClickHouse) | Target for Bronze tables | `p1` |

## 11. Assumptions

- The Cursor team account is on a Business or Enterprise plan with Admin API access
- The API key has been generated by a team admin from the Cursor dashboard
- The Cursor API response format remains stable across minor versions
- `email` and `userEmail` are stable, non-null fields across all endpoints
- Usage events are immutable except for cost adjustments within the current billing period
- The `POST` endpoints accept `startDate`/`endDate` as Unix timestamps in milliseconds in the JSON request body
- Daily usage data is available with minimal lag (same day or next day)
- The API returns zero-activity rows for all team members in the daily usage endpoint, even for dates with no activity
- No two usage events from the same user can share the same millisecond timestamp (required for the computed deduplication key `userEmail + timestamp`)

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| API key revoked or rotated by admin | All streams fail | Monitor sync status; alert on authentication failures; document key rotation procedure |
| Retroactive cost adjustments not captured | Stale cost data in Bronze; incorrect budget reports | Implement dual-schedule sync (hourly + daily billing-period resync) — see `cpt-insightspec-fr-cursor-dual-sync` |
| `tokenUsage` null for some events | Cost aggregation at Silver level must handle null tokens separately from zero-cost events | Document null semantics; Silver queries must treat null `tokenUsage` as "no token detail available", not "zero cost" |
| API rate limiting under large teams | Extraction takes longer; risk of timeout on high-volume event streams | Handle HTTP 429 with 60s retry delay; paginate with page size 500–1000 |
| Billing period boundary (27th of month) | Events near the boundary may shift between periods | Overlap sync windows across billing period boundaries |
| `isFreeBugbot` field not in original spec | Schema drift between documentation and actual API | Use actual API response as source of truth; update spec when discrepancies found |
| Daily usage returns zero-activity rows | Backfill probes backward and never terminates if checking only array length | Check actual metric values (not just row count) to determine earliest activity window |
| `cursor_collection_runs` stream not implemented in manifest | FR `cpt-insightspec-fr-cursor-collection-runs` and descriptor.yaml reference this stream, but connector.yaml does not include it | Implement the stream in the manifest, or remove from PRD scope and descriptor if monitoring is handled externally by the Airbyte platform |

## 13. Open Questions

Open questions are tracked in the DESIGN document ([DESIGN.md](./DESIGN.md) § Open Questions). Key items requiring resolution:

| ID | Summary | Owner | Target |
|----|---------|-------|--------|
| OQ-CUR-1 | Unified Silver stream (`class_ai_dev_usage`) — daily aggregate vs event-level | Data Architecture | Q2 2026 |
| OQ-CUR-2 | `tokenUsage` null semantics — when is token detail absent? | Connector Team | Q2 2026 |
| OQ-CUR-3 | Billing period boundary handling — overlap strategy | Connector Team | Q2 2026 |
| OQ-CUR-4 | Audit log event type taxonomy | Connector Team | Q3 2026 |
| OQ-CUR-5 | `maxMode` pricing implications | Data Architecture | Q3 2026 |

## 14. Non-Applicable Requirements

The following checklist domains have been evaluated and determined not applicable for this connector:

| Domain | Reason |
|--------|--------|
| **Security (SEC)** | The connector handles a single API key, marked `airbyte_secret` by the Airbyte framework. No custom authentication, authorization, or data protection logic exists in the declarative manifest. Security architecture is delegated to the Airbyte platform. |
| **Safety (SAFE)** | Pure data extraction pipeline. No interaction with physical systems, no potential for harm to people, property, or environment. |
| **Performance (PERF)** | Batch connector with native API pagination. No caching, pooling, or latency optimization needed. Rate limit handling (HTTP 429 retry) is the only performance concern, covered in §3.1. |
| **Reliability (REL)** | Idempotent extraction via deduplication keys. No distributed state, no transactions, no saga patterns. Recovery is handled by re-running the sync (Airbyte framework manages state). |
| **Usability (UX)** | No user-facing interface. Configuration is a single API key field in the Airbyte UI. |
| **Compliance (COMPL)** | Work emails are personal data under GDPR. Retention, deletion, and access controls are delegated to the Airbyte platform and destination operator. The connector must not store credentials outside the platform's secret management. Data residency is a platform responsibility. |
| **Maintainability (MAINT)** | Declarative YAML manifest — no custom code to maintain. Schema changes are handled by updating field definitions in the manifest. |
| **Testing (TEST)** | Connector behaviour must satisfy PRD acceptance criteria (§9). Validation includes: Airbyte framework connection check, schema validation, and connector-specific acceptance tests (verify `tenant_id` presence, stream completeness, pagination exhaustion). No custom unit tests required — the declarative manifest is validated by the framework. |
