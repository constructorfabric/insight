# PRD — Claude Enterprise Connector

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
  - [5.1 Per-User Activity Collection](#51-per-user-activity-collection)
  - [5.2 Organization Summary Collection](#52-organization-summary-collection)
  - [5.3 Chat Project Collection](#53-chat-project-collection)
  - [5.4 Skill Adoption Collection](#54-skill-adoption-collection)
  - [5.5 Connector Adoption Collection](#55-connector-adoption-collection)
  - [5.6 Connector Operations](#56-connector-operations)
  - [5.7 Data Integrity](#57-data-integrity)
  - [5.8 Identity Resolution](#58-identity-resolution)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Configure Claude Enterprise Connection](#configure-claude-enterprise-connection)
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

The Claude Enterprise connector extracts organization-wide engagement analytics from the Anthropic Enterprise Analytics API, including per-user activity across Claude (chat), Claude Code, Claude Cowork, and Office agents (Excel, PowerPoint); organization-wide active-user counts and seat utilization; chat project usage; and adoption metrics for skills and MCP connectors. It loads this data into the Insight platform's Bronze layer, enabling analytics teams to track how the organization actually uses its Claude Enterprise subscription across all product surfaces.

This connector complements the forthcoming `claude-admin` connector (which covers billing, token usage, workspace membership, and API-key management via the Anthropic Admin API). The Enterprise Analytics API is a distinct endpoint group at `https://api.anthropic.com/v1/organizations/analytics/` and requires a separately-scoped API key.

### 1.2 Background / Problem Statement

Organizations running Claude Enterprise have no centralized visibility into **how** their subscription is actually being used. Existing sources — the Admin API token/cost reports and the Claude Code usage endpoint — answer *how much* Claude is being invoked but not *who* is active, *which* products they use (chat vs Claude Code vs Cowork vs Office agents), which *skills* and *connectors* are adopted, and how *engagement* (DAU/WAU/MAU) evolves over time.

The Anthropic Enterprise Analytics API exposes this data through five endpoints, but it must be ingested, identity-resolved, and landed in the Bronze layer before it can inform internal dashboards, executive reports, or adoption reviews.

The Enterprise Analytics API provides five data areas:

- **Per-user activity** — per-user-per-day engagement counters across chat, Claude Code, Office agents, and Cowork
- **Organization summaries** — per-day DAU/WAU/MAU, assigned seat count, pending invites, and Cowork-specific active user counts
- **Chat projects** — per-project-per-day user, conversation, and message counts, plus project ownership
- **Skills** — per-skill-per-day adoption metrics across chat, Claude Code, Office agents, and Cowork
- **Connectors** — per-connector-per-day adoption metrics (normalized MCP/connector names) across the same surfaces

Unlike the Admin API connectors, the Enterprise Analytics API is **engagement-focused** (users, sessions, distinct counts), not **consumption-focused** (tokens, cost). The two sets of connectors complement each other and feed different analytics needs.

**Target Users**:

- Platform operators who obtain the Enterprise Analytics API key and monitor extraction runs
- People analytics and operations leaders who track adoption, seat utilization, and product-surface mix
- Data analysts who join Enterprise engagement metrics with IDE-tool metrics (Cursor, Windsurf, GitHub Copilot) and HR/organizational data

**Key Problems Solved**:

- Continuous extraction of Claude Enterprise engagement data into the Insight Bronze layer
- Visibility into per-user activity across every Claude product surface (chat, Code, Cowork, Office agents)
- Organization-level active-user and seat-utilization trends
- Skill and connector adoption signals, enabling product teams to see which capabilities are actually used
- Identity resolution via `email` enabling joins with HR systems, IDE-tool data, and other enterprise sources

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- Per-user activity and organization summaries extracted daily with complete coverage (Baseline: no extraction; Target: Q2 2026)
- Engagement data available for identity resolution within four days of activity, accounting for the Anthropic three-day reporting lag (Baseline: N/A; Target: Q2 2026)
- Skill and connector adoption metrics available for executive adoption reviews (Baseline: no data; Target: Q2 2026)

**Capabilities**:

- Extract per-user daily activity from `GET /v1/organizations/analytics/users`
- Extract organization-wide daily summaries from `GET /v1/organizations/analytics/summaries`
- Extract chat project activity from `GET /v1/organizations/analytics/apps/chat/projects`
- Extract skill adoption from `GET /v1/organizations/analytics/skills`
- Extract connector adoption from `GET /v1/organizations/analytics/connectors`
- Incremental extraction for all date-parameterized streams using a date-based cursor
- Identity resolution via `email` on the users stream and on `chat_projects.created_by`
- Respect the Anthropic minimum queryable date (January 1, 2026) and the three-day reporting lag

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Anthropic Enterprise Analytics API | Anthropic's REST API for Enterprise engagement analytics. Endpoints under `https://api.anthropic.com/v1/organizations/analytics/` provide per-user activity, organization summaries, chat project usage, and skill/connector adoption. Distinct from the Admin API. |
| Enterprise Analytics API Key | Authentication credential for the Enterprise Analytics API, scoped `read:analytics`, created at `claude.ai/analytics/api-keys` by a Primary Owner. Sent via `x-api-key` header. |
| Claude | The first-party Anthropic chat product (`claude.ai`), distinct from Claude Code and Cowork. |
| Claude Code | Anthropic's CLI- and IDE-based AI coding assistant. Usage is surfaced as `claude_code_metrics` on the per-user activity endpoint. |
| Claude Cowork | Anthropic's agentic coworker product. Usage is surfaced as `cowork_metrics` on the per-user activity endpoint and on summaries (`cowork_*_active_user_count`). |
| Office Agents | Claude's Excel and PowerPoint integrations. Surfaced as `office_metrics.excel` and `office_metrics.powerpoint`. |
| DAU / WAU / MAU | Daily / Weekly / Monthly Active Users. "Active" means: sent ≥1 chat message, or had a Claude Code session with tool/git activity, or had a Cowork session with tool or message activity. Weekly and monthly counts use rolling windows ending on the reference date. |
| Reporting Lag | The Enterprise Analytics API makes data for day `N-1` queryable only on day `N+2` (three full days after aggregation). Data for day `N` is therefore queryable starting on day `N+3`. |
| Cursor Pagination | Pagination mechanism used by the Enterprise Analytics API: clients pass `page=<opaque_token>` and receive a `next_page` field (string or `null`) in the response. Opaque to the client. |
| `person_id` | Canonical cross-system person identifier resolved by the Identity Manager. |
| `class_ai_*` | Silver streams for AI usage metrics. This connector feeds two of them: `class_ai_dev_usage` (Claude Code activity → `claude_enterprise__ai_dev_usage`) and `class_ai_assistant_usage` (chat / cowork / office → `claude_enterprise__ai_assistant_usage`). |
| Bronze Table | Raw data table in the destination, preserving source-native field names and types without transformation, with tenant tagging and provenance fields added. |
| `data_source` | Discriminator field set to `insight_claude_enterprise` in all Bronze rows emitted by this connector. |

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-insightspec-actor-claude-enterprise-operator`

**Role**: Obtains the Enterprise Analytics API key from an Organization Primary Owner, configures the connector, and monitors extraction runs.
**Needs**: Ability to configure the connector with the API key, tenant ID, start date, and (for local development only) an alternate base URL; verify that data flows correctly for all streams; and receive alerts on failed runs.

#### Organization Admin

**ID**: `cpt-insightspec-actor-claude-enterprise-admin`

**Role**: Manages the Claude Enterprise subscription, provisions seats, and monitors organization-wide engagement and adoption.
**Needs**: Visibility into DAU/WAU/MAU, seat utilization, pending invites, and the product-surface mix (chat vs Code vs Cowork vs Office agents) over time.

#### Data Analyst

**ID**: `cpt-insightspec-actor-claude-enterprise-analyst`

**Role**: Consumes Claude Enterprise engagement data from the Bronze layer (and, downstream, Silver/Gold) to build dashboards combining Claude activity with IDE-tool usage, HR data, and other enterprise signals.
**Needs**: Complete, gap-free engagement data with stable schemas and `email`-based identity resolution for cross-platform aggregation.

### 2.2 System Actors

#### Anthropic Enterprise Analytics API

**ID**: `cpt-insightspec-actor-claude-enterprise-anthropic-api`

**Role**: External REST API providing per-user activity, organization summaries, chat project usage, and skill/connector adoption. Enforces authentication via the `x-api-key` header with `read:analytics` scope, and enforces organization-level rate limits. Data is aggregated daily with a three-day reporting lag and is queryable only for dates on or after January 1, 2026.

#### Identity Manager

**ID**: `cpt-insightspec-actor-claude-enterprise-identity-mgr`

**Role**: Resolves `email` from Claude Enterprise Bronze tables to canonical `person_id` values in Silver step 2. Enables cross-system joins with IDE-tool connectors, HR/directory connectors, task trackers, and version control connectors.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires a Claude Enterprise account with the Enterprise Analytics API enabled, and an API key scoped `read:analytics` created at `claude.ai/analytics/api-keys` by a Primary Owner.
- Authentication uses the `x-api-key` header; the `anthropic-version: 2023-06-01` header **MUST** also be sent on all requests for consistency with other Anthropic APIs.
- The Enterprise Analytics API aggregates data once per day with a three-day reporting lag. Queries for dates less than three days in the past, or for dates before 2026-01-01, are rejected with HTTP 400. The connector **MUST NOT** emit requests that would fail this validation.
- The `/summaries` endpoint accepts a date range but caps the range at 31 days; the connector **MUST** chunk longer ranges across multiple requests.
- All other endpoints accept a single `date` parameter and return one snapshot per request.
- All paginated endpoints use cursor-based pagination (`limit` + `page`); default page sizes are 20 for `/users` and 100 for the remaining endpoints. Maximum is 1000.
- Rate limits are organization-level and adjustable with Anthropic CSM. The connector **MUST** honour HTTP 429 responses via exponential backoff.
- The connector **MUST** support a configurable base URL so operators can target a local stub service during development without code changes; the production default is `https://api.anthropic.com`.
- Bronze rows carry `collected_at` (§5.7) but the connector itself enforces **no retention, archival, purging, or GDPR Article 17 erasure policy**. Retention, deletion, and erasure of personal data (including `user_email` and `created_by_email`) are the responsibility of the destination operator and the Airbyte/warehouse platform. The policy itself is outside the scope of this connector.

## 4. Scope

### 4.1 In Scope

- Collection of per-user daily activity (chat, Claude Code, Office agents, Cowork, web search)
- Collection of daily organization summaries (DAU/WAU/MAU, seat counts, Cowork-specific active users)
- Collection of daily chat project activity (per-project user, conversation, and message counts plus ownership)
- Collection of daily skill adoption metrics across product surfaces
- Collection of daily connector (MCP) adoption metrics across product surfaces
- Connector execution monitoring via a collection-runs stream
- Identity resolution via `email` (users stream, chat-project ownership)
- Bronze-layer table schemas for all five data streams and the operations stream
- Configurable base URL override for local development and testing
- Incremental sync for all date-parameterized streams using a date-based cursor
- Enforcement of the Anthropic minimum queryable date (2026-01-01) and three-day reporting lag
- Silver-layer staging models routing Bronze → `class_ai_dev_usage` (Claude Code activity) and Bronze → `class_ai_assistant_usage` (chat / cowork / office) for orgs on the Enterprise subscription. Per the cross-vendor source-resolution rule, this connector becomes the canonical Code feed when present; Claude Admin's `claude_admin__ai_dev_usage` is left untagged for `class_ai_dev_usage` to avoid double-counting.

### 4.2 Out of Scope

- Admin API data (token usage, cost, API keys, workspaces, invites) — covered by the forthcoming `claude-admin` connector
- `class_ai_api_usage` (programmatic API tokens) — Enterprise does not expose token-level metering; this class is fed by `claude-admin` and future OpenAI staging
- `class_ai_cost` — Enterprise cost is org-level (subscription), not per-user; not surfaced in the Analytics API
- Silver step 2 identity resolution (`email` → `person_id`) — responsibility of the Identity Manager
- Gold-layer aggregations and cross-source productivity metrics
- Real-time or sub-daily granularity — the Enterprise Analytics API provides per-day aggregates only
- Per-request or per-conversation detail — the API returns daily aggregates per user/project/skill/connector
- Historical backfill before 2026-01-01 — rejected by the API
- Stub / mock service implementation — treated as a separate development aid; the connector sees the stub and the real API identically through the base-URL override

## 5. Functional Requirements

### 5.1 Per-User Activity Collection

#### Extract Per-User Daily Activity

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-enterprise-users-collect`

The connector **MUST** extract per-user daily activity records from the `GET /v1/organizations/analytics/users` endpoint, capturing `user.id` and `user.email_address`, the full `chat_metrics` counters, the full `claude_code_metrics` (core metrics and tool-action acceptance/rejection counts), `web_search_count`, `office_metrics.excel`, `office_metrics.powerpoint`, and `cowork_metrics`.

**Rationale**: Per-user activity is the primary signal for adoption and productivity analytics. It is the highest-cardinality stream and feeds per-person dashboards, manager reviews, and any eventual cross-tool productivity model.

**Actors**: `cpt-insightspec-actor-claude-enterprise-analyst`, `cpt-insightspec-actor-claude-enterprise-admin`

#### Incremental Sync for Per-User Activity

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-enterprise-users-incremental`

The connector **MUST** support incremental sync for per-user activity using a date-based cursor. On each run, only new days since the last cursor position are fetched. On first run, the connector fetches from a configurable `start_date` (default: 14 days ago, not earlier than 2026-01-01) forward to the newest queryable date permitted by the three-day reporting lag.

**Rationale**: Incremental sync avoids re-fetching the entire history on each run and makes daily syncs cheap. A shorter default start window than other connectors (14 days vs 90) reflects the restricted minimum queryable date and the operational cost of paginating per-user records.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

### 5.2 Organization Summary Collection

#### Extract Daily Organization Summary

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-enterprise-summaries-collect`

The connector **MUST** extract daily organization summaries from the `GET /v1/organizations/analytics/summaries` endpoint, capturing `date`, `daily_active_user_count`, `weekly_active_user_count`, `monthly_active_user_count`, `assigned_seat_count`, `pending_invite_count`, and the three `cowork_*_active_user_count` fields.

**Rationale**: Organization summaries feed executive-level adoption and seat-utilization reporting and establish the headline DAU/WAU/MAU trends over time.

**Actors**: `cpt-insightspec-actor-claude-enterprise-admin`, `cpt-insightspec-actor-claude-enterprise-analyst`

#### Chunk Summary Ranges to 31 Days

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-enterprise-summaries-chunking`

The connector **MUST** enforce the 31-day maximum range of the `/summaries` endpoint by issuing multiple requests when the cursor-to-now window exceeds 31 days.

**Rationale**: Attempting to request more than 31 days in a single call returns an HTTP 400 and aborts the stream. The connector must handle this transparently to the operator.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

### 5.3 Chat Project Collection

#### Extract Daily Chat Project Activity

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-enterprise-projects-collect`

The connector **MUST** extract daily chat project records from the `GET /v1/organizations/analytics/apps/chat/projects` endpoint, capturing `project_name`, `project_id`, `distinct_user_count`, `distinct_conversation_count`, `message_count`, `created_at`, and `created_by.id` / `created_by.email_address`.

**Rationale**: Chat projects are the most prominent collaboration artefact on `claude.ai` and reveal which initiatives are actually served by Claude. Ownership data (`created_by`) participates in identity resolution.

**Actors**: `cpt-insightspec-actor-claude-enterprise-admin`, `cpt-insightspec-actor-claude-enterprise-analyst`

#### Incremental Sync for Chat Projects

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-enterprise-projects-incremental`

The connector **MUST** support incremental sync for chat projects using the same date-based cursor semantics as the per-user activity stream (§5.1).

**Rationale**: Consistent cursor semantics across streams simplifies operation and avoids gaps.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

### 5.4 Skill Adoption Collection

#### Extract Daily Skill Adoption

- [ ] `p3` - **ID**: `cpt-insightspec-fr-claude-enterprise-skills-collect`

The connector **MUST** extract daily skill adoption records from the `GET /v1/organizations/analytics/skills` endpoint, capturing `skill_name`, `distinct_user_count`, and the per-surface session/conversation counts for chat, Claude Code, Office agents (Excel, PowerPoint), and Cowork.

**Rationale**: Skill adoption is the primary signal for product teams to judge which capabilities are actually used and should continue to be invested in.

**Actors**: `cpt-insightspec-actor-claude-enterprise-admin`, `cpt-insightspec-actor-claude-enterprise-analyst`

#### Incremental Sync for Skills

- [ ] `p3` - **ID**: `cpt-insightspec-fr-claude-enterprise-skills-incremental`

The connector **MUST** support incremental sync for skills using the same date-based cursor semantics as the per-user activity stream.

**Rationale**: Consistent cursor semantics across streams simplifies operation.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

### 5.5 Connector Adoption Collection

#### Extract Daily Connector Adoption

- [ ] `p3` - **ID**: `cpt-insightspec-fr-claude-enterprise-connectors-collect`

The connector **MUST** extract daily MCP/connector adoption records from the `GET /v1/organizations/analytics/connectors` endpoint, capturing `connector_name` (normalized by the API), `distinct_user_count`, and the per-surface session/conversation counts for chat, Claude Code, Office agents (Excel, PowerPoint), and Cowork.

**Rationale**: Connector adoption tells us which external systems (GitHub, Atlassian, Slack, etc.) are actually being reached from Claude and guides which MCP integrations the organization should prioritize or deprecate.

**Actors**: `cpt-insightspec-actor-claude-enterprise-admin`, `cpt-insightspec-actor-claude-enterprise-analyst`

#### Incremental Sync for Connectors

- [ ] `p3` - **ID**: `cpt-insightspec-fr-claude-enterprise-connectors-incremental`

The connector **MUST** support incremental sync for connector adoption using the same date-based cursor semantics as the per-user activity stream.

**Rationale**: Consistent cursor semantics across streams simplifies operation.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

### 5.6 Connector Operations

#### Track Collection Runs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-enterprise-collection-runs`

> **Phase 1 deferral**: The collection-runs stream is NOT emitted by the Airbyte connector manifest. In Phase 1, operational monitoring is provided by the Argo orchestrator pipeline, which produces one workflow run record per pipeline execution (capturing sync status, duration, and dbt outcome). The connector-level `collection_runs` stream with per-stream record counts and API call metrics is deferred to Phase 2 when richer instrumentation is needed. This deferral is consistent with the Confluence connector (same approach per DESIGN §3.7 note).

The connector **MUST** produce a collection-run log entry for each execution, recording `run_id`, `started_at`, `completed_at`, `status`, per-stream record counts, `api_calls`, `errors`, and `settings`.

**Rationale**: Operational visibility into connector health. Enables alerting on failed runs, quota watching, and data-completeness audits over time.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

#### Respect Minimum Queryable Date

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-enterprise-min-date-enforcement`

The connector **MUST NOT** emit requests whose `date`, `starting_date`, or `ending_date` resolves to a day before 2026-01-01. When a configured `start_date` precedes this minimum, the connector **MUST** clamp the effective cursor to 2026-01-01 and log the clamp.

**Rationale**: The Enterprise Analytics API rejects pre-2026 dates with HTTP 400, which would otherwise fail the entire sync.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

#### Respect Reporting Lag

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-enterprise-reporting-lag`

The connector **MUST NOT** emit requests for dates that fall within the Anthropic three-day reporting lag. The effective upper bound for any sync is `today() - 3 days`; later dates are deferred to the next run.

**Rationale**: Requests for dates still within the lag window return HTTP 400. Emitting them would mask genuine failures and inflate error counts. Holding them for the next run produces a complete and consistent Bronze landscape.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

### 5.7 Data Integrity

#### Deduplicate by Primary Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-enterprise-deduplication`

Each stream **MUST** use a primary key to ensure that re-running the connector for an overlapping date range does not produce duplicate records:

- `claude_enterprise_users`: key = `unique_key` (computed as `{date}:{user.id}`)
- `claude_enterprise_summaries`: key = `date`
- `claude_enterprise_chat_projects`: key = `unique_key` (computed as `{date}:{project_id}`)
- `claude_enterprise_skills`: key = `unique_key` (computed as `{date}:{skill_name}`)
- `claude_enterprise_connectors`: key = `unique_key` (computed as `{date}:{connector_name}`)
- `claude_enterprise_collection_runs`: key = `run_id`

**Rationale**: Incremental sync may revisit dates that have already been fetched (for example, if the API's lag window moved). Primary keys ensure idempotent extraction regardless.

**Actors**: `cpt-insightspec-actor-claude-enterprise-anthropic-api`

#### Tenant Tagging and Provenance

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-enterprise-tenant-tagging`

Every Bronze row **MUST** carry `tenant_id` (from connector configuration), `insight_source_id` (identifying the specific connector instance), `data_source = 'insight_claude_enterprise'`, and `collected_at` (UTC ISO-8601 timestamp of the extraction run).

**Rationale**: Tenant tagging is a platform-wide invariant for multi-tenant data isolation. Provenance fields make Bronze rows traceable back to a specific run and connector instance for debugging and auditing.

**Actors**: `cpt-insightspec-actor-claude-enterprise-operator`

### 5.8 Identity Resolution

#### Expose Identity Keys

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-enterprise-identity-key`

The `claude_enterprise_users` stream **MUST** include `user_email` as a non-null identity field. The `claude_enterprise_chat_projects` stream **MUST** include `created_by_email` as an identity field (may be null for system-created projects). These fields are used by the Identity Manager to resolve users to canonical `person_id` values in Silver step 2.

**Exemption**: The summaries stream is aggregated across all users and carries no per-user identity. The skills and connectors streams are aggregated per skill/connector and carry no per-user identity. The collection-runs stream is operational and carries no user identity.

**Rationale**: Cross-system identity resolution is the foundation of the Insight platform's analytics. Email is the stable, cross-platform identity key shared across Claude Enterprise, IDE tools, HR systems, and version control.

**Actors**: `cpt-insightspec-actor-claude-enterprise-identity-mgr`

#### Use Email as the Sole Identity Key

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-enterprise-identity-email-only`

The connector **MUST** treat `email` as the sole identity key for cross-system resolution. The Anthropic-internal `user.id` and `created_by.id` fields **MUST NOT** be used for cross-system identity resolution, though they **SHOULD** be retained in Bronze for debugging and for the rare case where two users share an email.

**Rationale**: Anthropic-platform IDs are meaningless outside the Anthropic ecosystem. Email is the stable cross-platform identity key.

**Actors**: `cpt-insightspec-actor-claude-enterprise-identity-mgr`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Data Freshness

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-claude-enterprise-freshness`

Engagement data for day `D` **MUST** be available in Bronze within 24 hours of becoming queryable in the API (i.e., by end of day `D + 4`, given the Anthropic three-day reporting lag).

**Threshold**: Bronze data for day `D` visible by end of day `D + 4`.

**Rationale**: The Anthropic three-day reporting lag is outside the connector's control. The connector's contribution is no more than one additional day beyond the API's own lag.

#### Extraction Completeness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-claude-enterprise-completeness`

The connector **MUST** extract 100% of records reported by the Enterprise Analytics API for each stream on each run, with zero record loss.

**Threshold**: Records extracted = records available in API (per stream, per date range).

**Rationale**: Partial extraction produces under-counted engagement metrics, which are particularly damaging for adoption reporting where the absolute numbers matter.

#### Schema Stability

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-claude-enterprise-schema-stability`

Bronze table schemas **MUST** remain stable across connector versions. Breaking schema changes **MUST** be versioned with migration guidance.

**Threshold**: Zero unannounced breaking changes to field names or types across all Bronze tables.

**Rationale**: Downstream consumers — including future Silver pipelines — depend on stable Bronze schemas. The Enterprise Analytics API itself is likely to add new surfaces over time; the connector **SHOULD** accommodate additive changes without breaking existing columns.

### 6.2 NFR Exclusions

- **Throughput / latency**: Not applicable — all streams are low-volume (hundreds to low thousands of records per day for a typical enterprise).
- **Availability**: Batch connector — availability is determined by the orchestrator's scheduling, not by this connector.
- **Cost accuracy**: Not applicable — the Enterprise Analytics API does not emit any cost or token-price fields. Cost analytics are delivered by the `claude-admin` connector.
- **Real-time delivery**: Not applicable — the API itself imposes a three-day lag, making sub-daily freshness impossible by design.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Claude Enterprise Stream Contract

- [ ] `p1` - **ID**: `cpt-insightspec-interface-claude-enterprise-streams`

**Type**: Data format (Bronze table schemas)

**Stability**: stable

**Description**: Six Bronze streams with defined schemas — `claude_enterprise_users`, `claude_enterprise_summaries`, `claude_enterprise_chat_projects`, `claude_enterprise_skills`, `claude_enterprise_connectors`, and `claude_enterprise_collection_runs`. The `users` stream carries `user_email` as its identity key; `chat_projects` carries `created_by_email`. All date-parameterized streams use a `date` cursor for incremental sync.

**Breaking Change Policy**: Adding new fields is non-breaking. Removing or renaming fields requires a migration.

### 7.2 External Integration Contracts

#### Anthropic Enterprise Analytics API

- [ ] `p1` - **ID**: `cpt-insightspec-contract-claude-enterprise-analytics-api`

**Direction**: required from external system

**Protocol/Format**: REST / JSON

| Stream | Endpoint | Method |
|--------|----------|--------|
| `claude_enterprise_users` | `GET /v1/organizations/analytics/users?date=YYYY-MM-DD` | Incremental |
| `claude_enterprise_summaries` | `GET /v1/organizations/analytics/summaries?starting_date=YYYY-MM-DD&ending_date=YYYY-MM-DD` | Incremental |
| `claude_enterprise_chat_projects` | `GET /v1/organizations/analytics/apps/chat/projects?date=YYYY-MM-DD` | Incremental |
| `claude_enterprise_skills` | `GET /v1/organizations/analytics/skills?date=YYYY-MM-DD` | Incremental |
| `claude_enterprise_connectors` | `GET /v1/organizations/analytics/connectors?date=YYYY-MM-DD` | Incremental |

**Authentication**: API key scoped `read:analytics`, sent via the `x-api-key` header. The `anthropic-version: 2023-06-01` header is also sent on all requests.

**Compatibility**: Enterprise Analytics API. Response format is JSON with cursor-based pagination (`limit` + `page` → `next_page`). Field additions are non-breaking. Data is queryable for dates on or after 2026-01-01 with a three-day reporting lag.

## 8. Use Cases

### Configure Claude Enterprise Connection

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-claude-enterprise-configure`

**Actor**: `cpt-insightspec-actor-claude-enterprise-operator`

**Preconditions**:

- Claude Enterprise account with Enterprise Analytics enabled
- Enterprise Analytics API key generated at `claude.ai/analytics/api-keys` by a Primary Owner

**Main Flow**:

1. Operator provides the Enterprise Analytics API key, tenant ID, and optionally a `start_date` and `insight_source_id`
2. System validates credentials against the Enterprise Analytics API (`GET /v1/organizations/analytics/summaries` for the most recent queryable day)
3. System initializes the connection with empty state

**Postconditions**:

- Connection is ready for first sync run

**Alternative Flows**:

- **Invalid or missing-scope API key**: API returns HTTP 404; system reports authentication failure; operator corrects the API key or scope
- **Pre-2026 `start_date`**: System accepts the configuration but clamps the effective cursor to 2026-01-01 and logs the clamp
- **Rate-limited on check**: System retries with exponential backoff; reports success if retry succeeds
- **Base URL override for development**: Operator supplies a development URL; system routes all requests through that URL instead of the production Anthropic endpoint

### Incremental Sync Run

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-claude-enterprise-incremental-sync`

**Actor**: `cpt-insightspec-actor-claude-enterprise-operator`

**Preconditions**:

- Connection configured and credentials valid
- Previous state available (or empty for first run)

**Main Flow**:

1. Orchestrator triggers the connector with current state
2. Connector computes the upper bound of the sync window as `today() - 3 days`
3. Connector iterates each date-parameterized stream (`users`, `chat_projects`, `skills`, `connectors`) day-by-day from last cursor to upper bound, paginating each day
4. Connector fetches `summaries` in chunks of up to 31 days from last cursor to upper bound
5. For each stream, pagination is exhausted before advancing the cursor; records are emitted with deduplication keys
6. State is updated with the last successfully-processed date for each stream
7. A `collection_runs` record is written with run metadata

**Postconditions**:

- Bronze tables contain new records for each stream
- State updated with per-stream cursor positions

**Alternative Flows**:

- **First run**: Cursor starts at configured `start_date` (clamped to 2026-01-01)
- **API throttling (HTTP 429)**: Connector retries with exponential backoff; reported in `collection_runs.errors`
- **Transient failure (HTTP 503)**: Connector retries with exponential backoff
- **Empty date range (cursor ≥ upper bound)**: Connector completes immediately with zero records
- **Stream-specific failure**: Other streams continue; failing stream is retried on next run

## 9. Acceptance Criteria

- Users stream extracts per-user daily records with `user_id`, `user_email`, `chat_metrics.*`, `claude_code_metrics.*`, `office_metrics.*`, `cowork_metrics.*`, and `web_search_count` populated
- Summaries stream extracts per-day records with DAU/WAU/MAU, seat counts, and Cowork active-user counts populated
- Chat projects stream extracts per-project-per-day records with `project_id`, `project_name`, `distinct_user_count`, `distinct_conversation_count`, `message_count`, and `created_by_email` populated
- Skills stream extracts per-skill-per-day records with `skill_name`, `distinct_user_count`, and per-surface session/conversation counts populated
- Connectors stream extracts per-connector-per-day records with `connector_name`, `distinct_user_count`, and per-surface session/conversation counts populated
- Incremental sync on second run extracts only new days (no duplicates)
- Pagination is exhausted for all paginated endpoints (no truncated results)
- Authentication with `x-api-key` header and `anthropic-version: 2023-06-01` works correctly
- A `start_date` earlier than 2026-01-01 is silently clamped to 2026-01-01
- Requests for dates within the three-day lag window are deferred, not emitted
- Summaries queries for ranges longer than 31 days are automatically split
- `tenant_id`, `insight_source_id`, `data_source = 'insight_claude_enterprise'`, and `collected_at` are present in every Bronze row
- A collection-run record is written for every execution, including runs that extracted zero records
- When the operator supplies a base URL override, all HTTP requests target that URL instead of `https://api.anthropic.com`

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Anthropic Enterprise Analytics API | Users, summaries, chat projects, skills, connectors endpoints | `p1` |
| Enterprise Analytics API key (`read:analytics` scope) | Authentication credential | `p1` |
| Airbyte Declarative Connector framework (latest) | Execution model for running the connector | `p1` |
| Identity Manager | Resolves `email` to `person_id` in Silver | `p2` |
| Destination store (PostgreSQL / ClickHouse) | Target for Bronze tables | `p1` |

## 11. Assumptions

- The organization is on a Claude Enterprise plan with Enterprise Analytics enabled
- The API key is created by a Primary Owner at `claude.ai/analytics/api-keys` with `read:analytics` scope
- The Enterprise Analytics API response format remains stable across minor revisions
- `user.email_address` is a stable, non-null field on the per-user endpoint for active users
- `created_by.email_address` on chat projects is stable for human-created projects (may be null for system-created projects)
- Daily data is queryable four days after its reference date (i.e., day `D` is available on day `D + 4`)
- Cursor pagination is opaque and the `next_page` token from one request is only ever used in the immediately following request for the same endpoint and date
- The `anthropic-version: 2023-06-01` header is tolerated by the Enterprise Analytics API (it is not documented as required, but is sent for consistency with other Anthropic APIs)
- The deploying organization is the data controller (GDPR Art. 4); Anthropic is the processor for the Enterprise Analytics API; the Airbyte platform and the destination operator are the joint processors for Bronze storage

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| API key revoked or scope reduced | All streams fail | Monitor sync status; alert on 404 authentication failures; document key rotation procedure |
| Rate limiting under heavy load (especially first-run backfill) | Sync takes longer; risk of timeout | Honor `Retry-After` on 429 with exponential backoff; paginate efficiently |
| Three-day lag window drifts or is reduced by Anthropic | Data freshness contract may tighten or loosen | Connector treats the lag as a configurable parameter (default 3 days) |
| Schema additions by Anthropic introduce fields the connector does not explicitly map | New fields may be dropped | Manifest allows additional properties; Bronze accepts unknown fields via `additionalProperties` |
| Data pre-2026-01-01 is requested | API returns HTTP 400; sync aborts | Clamp `start_date` to 2026-01-01; log the clamp |
| `summaries` range > 31 days requested | API returns HTTP 400 | Chunk requests into 31-day windows |
| Base-URL override is misconfigured in production | Requests go to the wrong host | Document override clearly; validate that production deployments omit the override |
| `anthropic-version` header becomes required or changes | Requests may be rejected if absent or outdated | Pin to `2023-06-01`; monitor Anthropic release notes |

## 13. Open Questions

Open questions for resolution during DESIGN:

| ID | Summary | Owner | Target |
|----|---------|-------|--------|
| OQ-CE-1 | Should the connector back-fill older dates on first run, or only forward from the most recent cursor? | Connector Team | Q2 2026 |
| OQ-CE-2 | How to expose Anthropic's opaque `project_id` format (`claude_proj_{id}`) to downstream consumers — keep verbatim or parse? | Data Architecture | Q2 2026 |
| OQ-CE-3 | Should the connector surface a single "active users" view that combines the `users` stream and the `summaries` stream, or keep them separate? | Data Architecture | Q3 2026 |
| OQ-CE-4 | How should the connector behave if the Enterprise Analytics API exposes additional surfaces (for example, beyond Office Excel/PowerPoint)? | Connector Team | Q3 2026 |
| OQ-CE-5 | Volume ceiling for a single connector instance — organization sizes beyond which the connector should be split (by workspace, by date partition, or otherwise). Current informal guidance: optimized for ≤10,000 seats per instance. | Connector Team | Q2 2026 |

## 14. Non-Applicable Requirements

The following checklist domains have been evaluated and determined not applicable for this connector:

| Domain | Reason |
|--------|--------|
| **Security (SEC)** | The connector handles a single API key, marked `airbyte_secret` by the Airbyte framework. No custom authentication, authorization, or data-protection logic exists in the declarative manifest. Security architecture is delegated to the Airbyte platform. |
| **Safety (SAFE)** | Pure data-extraction pipeline. No interaction with physical systems, no potential for harm to people, property, or environment. |
| **Performance (PERF)** | Batch connector with native API pagination. No caching, pooling, or latency optimization needed. Rate-limit handling (exponential backoff on 429) is the only performance concern, documented in SS3.1. |
| **Reliability (REL)** | Idempotent extraction via deduplication keys. No distributed state, no transactions, no saga patterns. Recovery is handled by re-running the sync (Airbyte framework manages state). |
| **Usability (UX)** | No user-facing interface. Configuration is a small set of fields (tenant_id, analytics_api_key, optional base URL, start date, source id) in the Airbyte UI. |
| **Compliance (COMPL)** | Work emails are personal data under GDPR. Retention, deletion, and access controls are delegated to the Airbyte platform and destination operator. The connector must not store credentials outside the platform's secret management. |
| **Maintainability (MAINT)** | Declarative YAML manifest — no custom code to maintain. Schema changes are handled by updating field definitions in the manifest. |
| **Testing — custom test tooling only** | Acceptance criteria (TEST-PRD-001) are documented in §9, and requirements use concrete, testable MUST/MUST NOT language (TEST-PRD-002). Only *custom test tooling* is N/A — the declarative manifest is validated by the Airbyte framework plus the §9 criteria, which are sufficient for acceptance without hand-written unit tests. |
| **Operations — deployment (OPS-PRD-001)** | Deployment is delegated to the orchestrator (Argo Workflows) per the Insight platform model. The connector itself has no deployment requirements beyond the Airbyte framework. |
| **Operations — monitoring (OPS-PRD-002)** | Connector-level monitoring is delegated to the orchestrator and destination. The connector contributes the `claude_enterprise_collection_runs` stream (§5.6) to support destination-side alerting. Dashboards, log retention, incident-response playbooks, and capacity monitoring are not defined at connector level. |
| **Integration — API requirements as producer (INT-PRD-002)** | The connector exposes no API. It is a pure data-extraction consumer of the Enterprise Analytics API; its integration obligations live entirely on the inbound side (§7.2). |
