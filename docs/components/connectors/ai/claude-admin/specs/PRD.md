# PRD — Claude Admin Connector

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
  - [5.1 Organization Directory Collection](#51-organization-directory-collection)
  - [5.2 API Usage and Cost Collection](#52-api-usage-and-cost-collection)
  - [5.3 Claude Code Usage Collection](#53-claude-code-usage-collection)
  - [5.4 API Key and Workspace Collection](#54-api-key-and-workspace-collection)
  - [5.5 Connector Operations](#55-connector-operations)
  - [5.6 Data Integrity](#56-data-integrity)
  - [5.7 Identity Resolution](#57-identity-resolution)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Configure Claude Admin Connection](#configure-claude-admin-connection)
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

The Claude Admin connector extracts organization-level administrative data from the Anthropic Admin API and loads it into the Insight Bronze layer. The connector covers:

- **Users / seats** — team member directory with role, status, and activity timestamps
- **Daily messages token usage** — programmatic API consumption per model / API key / workspace / service tier / context window
- **Daily cost report** — workspace and description-level cost attribution
- **Daily Claude Code usage** — developer AI tool activity (tokens, tool actions, sessions) per user per terminal
- **API keys** — key metadata with creator context and workspace assignment
- **Workspaces** — organizational workspace definitions and data residency
- **Workspace members** — user-to-workspace assignments and roles
- **Pending invites** — outstanding organization invitations

This connector consolidates the previously-separate `claude-api` and `claude-team` connectors. Both hit the same endpoint (`api.anthropic.com`) with the same credential (Admin API key via `x-api-key`), and each defined its own copy of `/v1/organizations/workspaces` and `/v1/organizations/invites`. Merging them removes the duplication, reduces operational surface area, and produces a single Bronze namespace (`bronze_claude_admin`) with 8 unique streams.

### 1.2 Background / Problem Statement

Organizations running Anthropic Claude at Team, Enterprise, or API scale have three complementary operational data surfaces:

- **Admin API — programmatic consumption**: token usage and cost by API key and workspace (formerly: `claude-api` connector)
- **Admin API — seat and developer activity**: users, workspaces, Claude Code usage by user (formerly: `claude-team` connector)
- **Enterprise Analytics API — engagement**: DAU/WAU/MAU, chat project usage, skill adoption, connector adoption (covered by the `claude-enterprise` connector — separate API, separate scope)

The first two used the same Admin API and the same credential, yet were shipped as two Airbyte manifests in two Bronze schemas with two duplicate endpoints. That created:

- Two K8s Secrets per deployment holding the same admin key.
- Two duplicate Bronze tables for workspaces and two for invites, differing only in trivial transformation details.
- Two dbt packages (`bronze_claude_api`, `bronze_claude_team`) with overlapping joins.
- Two PRD/DESIGN documents to keep aligned.

**Key Problems Solved**:

- Single connector package, single K8s Secret, single dbt source namespace for everything served by the Admin API.
- Deduplicated `workspaces` and `invites` streams — one canonical Bronze table per endpoint.
- Unified `insight_claude_admin` source discriminator — downstream queries stop needing to `UNION` two `data_source` values that represent the same external system.
- Aligned with the `claude-enterprise` connector's newer YAML structure (declarative manifest version 7.0.4, `error_handler` patterns), reducing cross-connector cognitive load.

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- All organization metadata (users, workspaces, workspace members, invites, API keys) extracted daily with complete coverage (Baseline: two separate connectors; Target: single unified connector Q2 2026)
- Daily API usage and cost available for cross-provider analytics with ≤ 48h latency (Baseline: 48h via claude-api; Target: unchanged)
- Daily Claude Code usage per user for developer AI adoption analytics with ≤ 48h latency (Baseline: 48h via claude-team; Target: unchanged)
- Single `data_source = 'insight_claude_admin'` discriminator replaces the prior `insight_claude_api` / `insight_claude_team` split (Baseline: two discriminators; Target: one)

**Capabilities**:

- Extract seat roster from `GET /v1/organizations/users`
- Extract daily token usage from `GET /v1/organizations/usage_report/messages` with full dimensional breakdown
- Extract daily cost report from `GET /v1/organizations/cost_report` with workspace and description breakdown
- Extract daily Claude Code usage from `GET /v1/organizations/usage_report/claude_code`
- Extract API keys, workspaces, workspace members (substream), and pending invites
- Incremental sync on `date` cursor for usage / cost / code usage streams; full refresh for dimension streams
- Identity resolution via `email` (users stream, invites) and `actor_identifier` (code usage, when `actor_type = 'user'`)

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Anthropic Admin API | Anthropic's REST API for organization administration. Endpoints under `https://api.anthropic.com/v1/organizations/` provide user management, usage reports, cost reports, API key metadata, workspace operations, and invite data. |
| Admin API Key | Authentication credential for the Anthropic Admin API. Sent via the `x-api-key` header with `anthropic-version: 2023-06-01`. Distinct from a standard ("user") API key — requires organization admin scope. |
| Seat | An assigned Claude subscription slot for a specific user (relevant for Team/Enterprise plans). |
| Claude Code | Anthropic's CLI and IDE-based AI coding assistant. Usage is captured daily per user via `/v1/organizations/usage_report/claude_code`. |
| `actor_identifier` | The `email` address of a user in Claude Code usage reports (when `actor_type = 'user'`), or the API key name when `actor_type = 'api_actor'`. Identity-resolvable only for the user case. |
| `api_key_id` | API key identifier assigned by Anthropic, used as the grouping dimension in messages usage. |
| `workspace_id` | Workspace identifier within the Anthropic organization. |
| `service_tier` | API service tier (`scale`, `standard`) affecting pricing and rate limits. |
| `context_window` | Maximum context window size used for the request batch. |
| `person_id` | Canonical cross-system person identifier resolved by the Identity Manager. |
| `class_ai_api_usage` | Silver stream for programmatic API usage (Claude Admin messages + OpenAI API), fed by `claude_admin__ai_api_usage` dbt model. |
| `class_ai_dev_usage` | Silver stream for developer AI tool usage (Claude Code, Cursor, Windsurf), fed by `claude_admin__ai_dev_usage` dbt model. |
| `data_source` | Discriminator field set to `insight_claude_admin` in all Bronze rows emitted by this connector. |

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-insightspec-actor-claude-admin-operator`

**Role**: Obtains the Anthropic Admin API key from an organization admin, configures the connector, monitors extraction runs, and handles credential rotation.
**Needs**: Single configuration surface (one K8s Secret, one connection) that covers both API usage and seat/Claude-Code usage; clear error reporting on authentication or rate-limit failures.

#### Organization Admin

**ID**: `cpt-insightspec-actor-claude-admin-admin`

**Role**: Manages the organization's Anthropic subscription and provisions seats, API keys, and workspaces.
**Needs**: Visibility into seat utilization, inactive seats, API-key ownership, workspace structure, pending invitations, and overall adoption trends.

#### Data Analyst

**ID**: `cpt-insightspec-actor-claude-admin-analyst`

**Role**: Consumes Admin Bronze data from Silver/Gold layers to build cost reports, API-spend dashboards, and AI dev tool adoption analyses combining Claude Code with Cursor, Windsurf, and other tools.
**Needs**: Complete, gap-free Bronze data with stable schemas and `email`-based identity resolution for cross-platform aggregation.

#### Finance / Engineering Manager

**ID**: `cpt-insightspec-actor-claude-admin-manager`

**Role**: Consumes Gold-layer reports aggregating Claude spend and usage alongside other AI API providers.
**Needs**: Accurate cost attribution by workspace, model, API key; trend visibility; reliable per-key owner attribution.

### 2.2 System Actors

#### Anthropic Admin API

**ID**: `cpt-insightspec-actor-claude-admin-anthropic-api`

**Role**: External REST API providing user management, usage reports, cost reports, API key metadata, workspace definitions, membership, and invite data. Enforces rate limits and requires API key authentication via `x-api-key` header with `anthropic-version: 2023-06-01`.

#### Identity Manager

**ID**: `cpt-insightspec-actor-claude-admin-identity-mgr`

**Role**: Resolves `email` from users / invites and `actor_identifier` from code usage to canonical `person_id` in Silver step 2. Also resolves API-key creators (`created_by_id` / `created_by_name`) when `created_by_type = 'user'`. Enables cross-system joins with IDE-tool connectors, HR/directory connectors, version control, and task trackers.

#### ETL Scheduler / Orchestrator

**ID**: `cpt-insightspec-actor-claude-admin-scheduler`

**Role**: Triggers connector runs on a configured schedule (default: daily at 02:00 UTC) and monitors collection run outcomes.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires outbound HTTPS access to `api.anthropic.com`.
- Authentication via Admin API key passed in the `x-api-key` header, with `anthropic-version: 2023-06-01` on every request.
- The Admin API key **MUST** have organization-level read scope (users, usage, cost, api_keys, workspaces, invites).
- The connector operates in batch pull mode; no inbound network port or webhook endpoint required.
- Usage and cost endpoints enforce a **maximum 31-day date range per request**. The connector chunks longer lookback at `P1D` granularity (one day per request) to avoid the boundary-day loss caused by Airbyte's inclusive-inclusive cursor arithmetic when `step: P31D` is combined with the API's exclusive `ending_at` semantics.
- Code usage (`/usage_report/claude_code`) uses `starting_at` (date-only) and does not accept `ending_at` or `bucket_width`.
- All endpoints are HTTP GET — no mutation calls.
- Pagination varies: `after_id` cursor (users), `next_page` cursor (messages usage, cost report, code usage), offset/limit (api_keys, workspaces, invites), iterated per-workspace (workspace members via `SubstreamPartitionRouter`).

## 4. Scope

### 4.1 In Scope

- Collection of current seat assignments (users stream: role, status, activity timestamps)
- Collection of daily API messages usage with full dimensional breakdown (model, API key, workspace, service tier, context window, inference geo, speed)
- Collection of daily cost reports with workspace and description breakdown
- Collection of daily Claude Code usage per user and per terminal type
- Collection of API keys with creator context (`created_by_id`, `created_by_name`, `created_by_type`)
- Collection of workspace definitions and per-workspace membership (substream)
- Collection of pending organization invites
- Incremental sync for usage / cost / code usage using date-based cursor at `P1D` step
- Full refresh for users, API keys, workspaces, workspace members, and invites
- Identity resolution via `email` (users, invites) and `actor_identifier` (code usage where `actor_type = 'user'`)
- Bronze-layer table schemas for all 8 data streams
- Two Silver-layer dbt models: `claude_admin__ai_api_usage` (feeds `class_ai_api_usage`) and `claude_admin__ai_dev_usage` (feeds `class_ai_dev_usage`)

### 4.2 Out of Scope

- Enterprise Analytics API (per-user activity, DAU/WAU/MAU, chat projects, skills, connectors) — covered by the `claude-enterprise` connector
- Silver step 2 (identity resolution: email → `person_id`) — responsibility of the Identity Manager
- Gold-layer aggregations and cross-source productivity metrics
- Web/mobile Claude usage metrics — the Admin API `usage_report/claude_code` endpoint is specifically for Claude Code; web/mobile activity is not exposed by the Admin API
- Per-request event collection — the Admin API provides daily aggregates only
- Real-time or sub-daily granularity — the Admin API provides daily aggregates only
- Versioning or history of seat assignment changes (current-state only, per API behaviour)
- `class_ai_tool_usage` Silver target — the prior `claude-team` connector shipped a placeholder model; the data is not available from this API and the placeholder is not carried forward
- Class-level Silver tags (`silver:class_ai_api_usage`, `silver:class_ai_dev_usage`) — tagging will be added in a separate PR alongside the Silver framework changes

## 5. Functional Requirements

### 5.1 Organization Directory Collection

#### Extract Users (Seat Roster)

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-users-collect`

The connector **MUST** extract all current seat assignments from `GET /v1/organizations/users`, capturing each user's `id`, `email`, `name`, `role` (owner / admin / user), `status` (active / inactive / pending), `added_at`, and `last_active_at`.

**Rationale**: The seat roster enables utilization reporting and provides the `email` identity key for cross-system resolution.
**Actors**: `cpt-insightspec-actor-claude-admin-admin`, `cpt-insightspec-actor-claude-admin-analyst`

#### Extract Pending Invitations

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-admin-invites-collect`

The connector **MUST** extract pending invitations from `GET /v1/organizations/invites`, capturing `id`, `email`, `role`, `status`, `created_at`, `expires_at`, and `workspace_id`.

**Rationale**: Invitations complement the seat roster by showing planned but not-yet-accepted seats. The field `created_at` is used (not `invited_at` from the prior `claude-team` schema) to align with the project-wide convention that timestamps of record creation are named `created_at`.
**Actors**: `cpt-insightspec-actor-claude-admin-admin`

### 5.2 API Usage and Cost Collection

#### Collect Messages Usage Reports

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-messages-usage`

The connector **MUST** collect daily messages usage records from `GET /v1/organizations/usage_report/messages?group_by[]=model&group_by[]=api_key_id&group_by[]=workspace_id&group_by[]=service_tier&group_by[]=context_window`, capturing `date`, `model`, `api_key_id`, `workspace_id`, `service_tier`, `context_window`, `inference_geo`, `speed`, `uncached_input_tokens`, `cache_read_tokens`, `cache_creation_5m_tokens`, `cache_creation_1h_tokens`, `output_tokens`, and `web_search_requests` at one row per unique dimension combination per day.

**Rationale**: Messages usage is the primary signal for API token consumption and cost attribution across organizational dimensions. `inference_geo` and `speed` are collected as nullable fields but are not part of the `group_by` dimensions because the API caps `group_by` at 5 values.
**Actors**: `cpt-insightspec-actor-claude-admin-analyst`

#### Collect Cost Reports

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-cost-report`

The connector **MUST** collect daily cost reports from `GET /v1/organizations/cost_report?group_by[]=workspace_id&group_by[]=description`, capturing `date`, `workspace_id`, `description`, `amount`, `currency`, `cost_type`, `model`, `service_tier`, `context_window`, `token_type`, and `inference_geo` at one row per `(date, workspace_id, description)`.

**Rationale**: Cost reports provide financial attribution complementing token-level usage data. The API returns `amount` as a string (not `amount_cents` as an integer) — the Bronze schema preserves this verbatim.
**Actors**: `cpt-insightspec-actor-claude-admin-analyst`, `cpt-insightspec-actor-claude-admin-manager`

#### Support Date-Range Incremental Sync for Usage and Cost

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-usage-incremental`

The connector **MUST** support incremental sync for messages usage and cost reports using `starting_at` and `ending_at` ISO 8601 parameters. The cursor field is `date`, the step is `P1D`, and `cursor_granularity` is `PT1S` to prevent the empty-window bug (`starting_at == ending_at` rejected by the API with HTTP 400).

**Rationale**: Incremental sync minimizes API calls. The `P1D` + `PT1S` combination is the known-good configuration inherited from the prior `claude-api` connector ADR-003.
**Actors**: `cpt-insightspec-actor-claude-admin-operator`

### 5.3 Claude Code Usage Collection

#### Extract Daily Claude Code Usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-code-usage-collect`

The connector **MUST** extract daily Claude Code usage from `GET /v1/organizations/usage_report/claude_code`, capturing `date`, `actor_type`, `actor_identifier` (email for users; API key name for `api_actor`), `terminal_type`, and the flattened / preserved fields: `session_count`, `lines_added`, `lines_removed`, `tool_use_accepted`, `tool_use_rejected`, plus `core_metrics_json`, `tool_actions_json`, `model_breakdown_json`.

**Rationale**: Claude Code usage is the primary data source feeding `class_ai_dev_usage` alongside Cursor and Windsurf. Per-model token data is preserved in `model_breakdown_json` for downstream flattening; Bronze remains source-native.
**Actors**: `cpt-insightspec-actor-claude-admin-analyst`, `cpt-insightspec-actor-claude-admin-admin`

#### Incremental Sync for Code Usage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-code-usage-incremental`

The connector **MUST** support incremental sync for code usage using `starting_at` (date-only, `YYYY-MM-DD`). The endpoint does **not** accept `ending_at` or `bucket_width`; the connector sends only `starting_at` and expects all usage for that date in the response. The cursor field is `date`, step is `P1D`, first-run start is a configurable `start_date` (default 90 days ago).

**Rationale**: The endpoint's input schema requires date-only; full datetime format is unsupported. Incremental sync avoids re-fetching history on each run.
**Actors**: `cpt-insightspec-actor-claude-admin-operator`

### 5.4 API Key and Workspace Collection

#### Collect API Key Metadata

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-keys`

The connector **MUST** collect API key metadata from `GET /v1/organizations/api_keys`, capturing `id`, `name`, `status`, `created_at`, `created_by_id` / `created_by_name` / `created_by_type` (flattened from nested `created_by`), `workspace_id`, and `partial_key_hint`.

**Rationale**: API key metadata enables per-key attribution of usage and identifies the creator of each key.
**Actors**: `cpt-insightspec-actor-claude-admin-analyst`

#### Collect Workspace Definitions

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-admin-workspaces`

The connector **MUST** collect workspace definitions from `GET /v1/organizations/workspaces`, capturing `id`, `name`, `display_name`, `created_at`, `archived_at`, and `data_residency` (nested object, serialized to JSON string with empty-string fallback when absent).

**Rationale**: Workspaces provide dimensional enrichment for cost and usage streams.
**Actors**: `cpt-insightspec-actor-claude-admin-analyst`

#### Collect Workspace Members

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-admin-workspace-members-collect`

The connector **MUST** extract workspace membership by iterating over all workspaces and fetching members from `GET /v1/organizations/workspaces/{id}/members`. Each record captures `user_id`, `workspace_id`, `workspace_role`. The primary key is composite: `{user_id}:{workspace_id}`.

**Rationale**: Workspace membership enables per-workspace utilization analysis and access auditing. Implemented as a `SubstreamPartitionRouter` whose parent is the workspaces stream.
**Actors**: `cpt-insightspec-actor-claude-admin-admin`

### 5.5 Connector Operations

#### Track Collection Runs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-admin-collection-runs`

> **Phase 1 deferral**: The `claude_admin_collection_runs` stream is **not** emitted by the Airbyte connector manifest. In Phase 1, operational monitoring is provided by the Argo orchestrator pipeline (one workflow run record per pipeline execution). Per-stream record counts and API call metrics are deferred to Phase 2, consistent with the `claude-enterprise` and `confluence` connectors.

The connector **MUST** produce a collection-run log entry for each execution, recording `run_id`, `started_at`, `completed_at`, `status`, per-stream record counts, `api_calls`, `errors`, and `settings`.

**Rationale**: Operational visibility into connector health. Enables alerting on failed runs and tracking data completeness over time.
**Actors**: `cpt-insightspec-actor-claude-admin-operator`

### 5.6 Data Integrity

#### Deduplicate by Primary Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-deduplication`

Each stream **MUST** use a primary key to ensure that re-running the connector for an overlapping date range does not produce duplicate records:

- `claude_admin_users`: key = `id`
- `claude_admin_messages_usage`: key = `unique` (composite: `date|model|api_key_id|workspace_id|service_tier|context_window`)
- `claude_admin_cost_report`: key = `unique` (composite: `date|workspace_id|description`)
- `claude_admin_code_usage`: key = `unique` (composite: `date|actor_type|actor_identifier|terminal_type`)
- `claude_admin_api_keys`: key = `id`
- `claude_admin_workspaces`: key = `id`
- `claude_admin_workspace_members`: key = `unique` (computed as `{user_id}:{workspace_id}`)
- `claude_admin_invites`: key = `id`

**Rationale**: Incremental sync may revisit dates that have already been fetched. Primary keys ensure idempotent extraction.
**Actors**: `cpt-insightspec-actor-claude-admin-anthropic-api`

#### Tenant Tagging and Provenance

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-tenant-tagging`

Every Bronze row **MUST** carry `tenant_id` (from connector configuration), `insight_source_id` (identifying the specific connector instance; default empty string), `data_source = 'insight_claude_admin'`, and `collected_at` (UTC ISO-8601 timestamp of the extraction run).

**Rationale**: Tenant tagging is a platform-wide invariant for multi-tenant isolation. The unified `insight_claude_admin` discriminator replaces the prior `insight_claude_api` / `insight_claude_team` split.
**Actors**: `cpt-insightspec-actor-claude-admin-operator`

### 5.7 Identity Resolution

#### Expose Identity Keys

- [ ] `p1` - **ID**: `cpt-insightspec-fr-claude-admin-identity-key`

The `claude_admin_users` stream **MUST** include `email` as a non-null identity field. The `claude_admin_code_usage` stream **MUST** include `actor_identifier` — which holds the user email when `actor_type = 'user'` and the API key name when `actor_type = 'api_actor'`. These fields are used by the Identity Manager to resolve users to canonical `person_id` values in Silver step 2.

**Exemption**: Monitoring and purely organizational streams (`claude_admin_workspaces`, `claude_admin_workspace_members`, `claude_admin_api_keys`, collection runs) do not carry a primary email identity field, though `claude_admin_api_keys.created_by_id` / `created_by_name` and `claude_admin_invites.email` are secondary identity resolution inputs.

**Rationale**: Email is the stable, cross-platform identity key shared across Claude Admin, IDE tools, HR systems, and version control.
**Actors**: `cpt-insightspec-actor-claude-admin-identity-mgr`

#### Use Email as the Sole Identity Key

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-admin-identity-email-only`

The connector **MUST** treat `email` / `actor_identifier` (when `actor_type = 'user'`) as the sole cross-system identity key. Anthropic's internal `id` (user ID) **MUST NOT** be used for cross-system identity resolution, though it is retained in Bronze for debugging.

**Rationale**: Anthropic-platform IDs are meaningless outside the Anthropic ecosystem. Email is the stable cross-platform identity key.
**Actors**: `cpt-insightspec-actor-claude-admin-identity-mgr`

#### Filter Claude Code Usage to `actor_type = 'user'` for Identity Resolution

- [ ] `p2` - **ID**: `cpt-insightspec-fr-claude-admin-code-usage-user-filter`

The Silver `claude_admin__ai_dev_usage` model **MUST** filter Bronze code usage rows to `actor_type = 'user'` before emitting identity-keyed rows. `api_actor` rows are retained in Bronze for completeness but are not routed to Silver identity resolution (they have no person).

**Rationale**: `actor_identifier` is an email only when `actor_type = 'user'`. For `api_actor`, it is an API key name — unresolvable to a person.
**Actors**: `cpt-insightspec-actor-claude-admin-identity-mgr`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Authentication via Admin API Key

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-claude-admin-auth`

The connector **MUST** authenticate using the `x-api-key` header with the provided Admin API key. All requests **MUST** include `anthropic-version: 2023-06-01`.

#### Rate Limit Compliance

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-claude-admin-rate-limiting`

The connector **MUST** implement exponential backoff on HTTP 429 responses and **SHOULD** honour `Retry-After` when present. Transient 5xx errors **MUST** trigger retry with exponential backoff.

#### Data Freshness

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-claude-admin-freshness`

Usage, cost, and code usage data for day `D` **MUST** be available in Bronze within 48 hours of end-of-day `D`. Full-refresh dimension streams (users, workspaces, members, keys, invites) **MUST** reflect the organization's state as of the last sync run.

#### Data Source Discriminator

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-claude-admin-data-source`

All rows written by this connector **MUST** carry `data_source = 'insight_claude_admin'`.

#### Idempotent Writes

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-claude-admin-idempotent`

Repeated collection of the same date range **MUST NOT** create duplicate rows. The connector **MUST** use upsert semantics keyed on `unique` (usage / cost / code usage / workspace members) or natural `id` (users / keys / workspaces / invites).

#### Schema Stability

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-claude-admin-schema-stability`

Bronze table schemas **MUST** remain stable across connector versions. Additive changes (new fields) are non-breaking. Removing or renaming fields requires a migration. The Bronze namespace is `bronze_claude_admin`; stream names use the `claude_admin_` prefix.

### 6.2 NFR Exclusions

- **Real-time latency SLA**: Not applicable — batch pull mode only.
- **GPU / high-compute NFRs**: Not applicable — I/O-bound API collection.
- **Safety (SAFE)**: Not applicable — data collection pipeline with no physical or safety-critical interactions.
- **Usability / UX**: Not applicable — the connector is configured via K8s Secret and Airbyte form.
- **Availability SLA (REL)**: Not applicable — scheduled batch job; availability is delegated to the orchestrator.
- **Regulatory compliance (COMPL)**: Work emails are personal data; retention, deletion, and access controls are delegated to the destination operator. The connector itself enforces no retention policy.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Claude Admin Stream Contract

- [ ] `p1` - **ID**: `cpt-insightspec-interface-claude-admin-streams`

**Type**: Data format (Bronze table schemas)

**Stability**: stable

**Description**: Eight Bronze streams with defined schemas — `claude_admin_users`, `claude_admin_messages_usage`, `claude_admin_cost_report`, `claude_admin_code_usage`, `claude_admin_api_keys`, `claude_admin_workspaces`, `claude_admin_workspace_members`, `claude_admin_invites`. Identity keys: `claude_admin_users.email` (primary), `claude_admin_code_usage.actor_identifier` where `actor_type = 'user'` (secondary). Incremental streams use `date` cursor.

**Breaking Change Policy**: Adding new fields is non-breaking. Removing or renaming fields requires a migration. Bronze namespace and stream names are stable (`bronze_claude_admin.claude_admin_*`).

### 7.2 External Integration Contracts

#### Anthropic Admin API

- [ ] `p1` - **ID**: `cpt-insightspec-contract-claude-admin-anthropic-api`

**Direction**: required from external system

**Protocol/Format**: REST / JSON

| Stream | Endpoint | Method | Sync Mode |
|--------|----------|--------|-----------|
| `claude_admin_users` | `GET /v1/organizations/users?limit=100` | GET | Full refresh |
| `claude_admin_messages_usage` | `GET /v1/organizations/usage_report/messages?group_by[]=...&starting_at=...&ending_at=...` | GET | Incremental |
| `claude_admin_cost_report` | `GET /v1/organizations/cost_report?group_by[]=workspace_id&group_by[]=description&starting_at=...&ending_at=...` | GET | Incremental |
| `claude_admin_code_usage` | `GET /v1/organizations/usage_report/claude_code?starting_at=YYYY-MM-DD` | GET | Incremental |
| `claude_admin_api_keys` | `GET /v1/organizations/api_keys?limit=1000` | GET | Full refresh |
| `claude_admin_workspaces` | `GET /v1/organizations/workspaces?limit=1000` | GET | Full refresh |
| `claude_admin_workspace_members` | `GET /v1/organizations/workspaces/{id}/members?limit=100` | GET | Full refresh (substream) |
| `claude_admin_invites` | `GET /v1/organizations/invites?limit=1000` | GET | Full refresh |

**Authentication**: `x-api-key: {admin_api_key}` + `anthropic-version: 2023-06-01`.

**Compatibility**: Anthropic Admin API. Response format is JSON. Pagination varies per endpoint (cursor `next_page` / `after_id` / offset). Field additions are non-breaking. `anthropic-version` is pinned to `2023-06-01`.

#### Identity Manager

- [ ] `p2` - **ID**: `cpt-insightspec-contract-claude-admin-identity-mgr`

**Direction**: required from client (Identity Manager service)

**Protocol/Format**: Internal service call; input is `email` / `name` / `source_label = "claude_admin"`; output is canonical `person_id` or NULL.

**Compatibility**: Identity Manager must be available during Silver pipeline execution. Unresolved identities remain with `person_id = NULL` and do not block Silver writes.

## 8. Use Cases

### Configure Claude Admin Connection

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-claude-admin-configure`

**Actor**: `cpt-insightspec-actor-claude-admin-operator`

**Preconditions**:

- Organization has an Anthropic plan with Admin API access.
- Organization admin has generated an Admin API key at console.anthropic.com.

**Main Flow**:

1. Operator creates a K8s Secret named `insight-claude-admin-{instance}` in the `data` namespace with `stringData.admin_api_key` set.
2. Operator optionally sets `start_date` in `stringData` for a non-default backfill window.
3. Orchestrator picks up the Secret via the `insight.cyberfabric.com/connector: claude-admin` annotation.
4. On first sync, the connector validates credentials by reading `claude_admin_workspaces` (the `check` stream).
5. On success, the connection is saved and scheduled.

**Postconditions**:

- Connection is configured and ready for scheduled or manual sync.

**Alternative Flows**:

- **Invalid API key**: Check fails with HTTP 401 / 403; operator corrects the Secret.
- **Insufficient scope**: Check fails (403 or 404); operator requests organization admin to elevate the key.

### Incremental Sync Run

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-claude-admin-incremental-sync`

**Actor**: `cpt-insightspec-actor-claude-admin-scheduler`

**Preconditions**:

- At least one prior successful sync exists (or `start_date` configured for first run).
- Admin API key is valid and has organization-level read scope.

**Main Flow**:

1. Scheduler triggers a sync run (default: daily at 02:00 UTC).
2. Full-refresh streams (users, api_keys, workspaces, workspace_members, invites): connector fetches all pages with configured pagination.
3. Incremental streams (messages_usage, cost_report, code_usage): connector reads cursor state, windows the date range at `P1D` step, and fetches all pages per window.
4. All records pass through `AddFields` transformations (tenant_id, insight_source_id, collected_at, data_source, and stream-specific composite keys) before hitting the destination.
5. Cursor state is persisted after each stream completes.

**Postconditions**:

- All Bronze tables are up-to-date through the last successful cursor date.

**Alternative Flows**:

- **API rate limiting (HTTP 429)**: Connector backs off exponentially and retries.
- **Partial failure**: Failed streams are retried; successful streams retain their data.
- **Empty code-usage day**: API returns empty results; connector emits zero records and advances the cursor.

## 9. Acceptance Criteria

- All 8 Bronze streams are populated on first run against a live Anthropic organization with `tenant_id`, `insight_source_id`, `data_source = 'insight_claude_admin'`, and `collected_at` on every row.
- `claude_admin_users.email` is non-null for all active seats.
- `claude_admin_code_usage.actor_identifier` is non-null; `actor_type` distinguishes `user` from `api_actor`.
- `claude_admin_messages_usage.unique` deduplicates correctly across overlapping incremental syncs (no duplicate composite keys).
- `claude_admin_cost_report.unique` deduplicates correctly.
- `claude_admin_workspace_members` produces one row per `(user_id, workspace_id)` via the `SubstreamPartitionRouter`.
- `claude_admin_invites.created_at` is present (not `invited_at` — that field name is not used).
- `claude_admin_workspaces.data_residency` is serialized as a JSON string with `''` fallback when absent.
- A second sync run for an overlapping date range completes without creating duplicate rows.
- Full-refresh streams correctly overwrite stale data.
- Pagination is exhausted for all paginated endpoints (no truncated results).
- `check` succeeds against `claude_admin_workspaces` with a valid key.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Anthropic Admin API | Source data for all 8 streams | `p1` |
| Admin API key (organization read scope) | Authentication credential | `p1` |
| Airbyte Declarative Connector framework (≥ 7.0.4) | Execution runtime for the manifest | `p1` |
| Identity Manager | Resolves `email` / `actor_identifier` to `person_id` in Silver step 2 | `p2` |
| Destination store (ClickHouse / PostgreSQL) | Target for Bronze tables | `p1` |
| dbt (Silver transformation runtime) | Executes `claude_admin__ai_api_usage` and `claude_admin__ai_dev_usage` models | `p2` |

## 11. Assumptions

- The organization has an Anthropic plan with Admin API access (Team, Enterprise, or API organization tier).
- The Admin API key has been generated by an organization admin at console.anthropic.com with organization-level read scope.
- The Anthropic Admin API response format remains stable across minor revisions; `anthropic-version: 2023-06-01` remains valid.
- `email` is a stable, non-null field on the users endpoint for active seats.
- `actor_identifier` in code usage holds an email when `actor_type = 'user'` and an API key name when `actor_type = 'api_actor'`.
- The 31-day maximum date range per request for usage / cost endpoints is a stable API constraint.
- The `claude_code` endpoint does not accept `ending_at` or `bucket_width` — only `starting_at`.
- Daily usage data is finalized with D+1 lag; no retroactive corrections to previously-synced dates (unlike the Enterprise Analytics API).

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| API key revoked or rotated by admin | All streams fail with 401 | Monitor sync status; alert on authentication failures; document key rotation procedure |
| Admin API rate limiting for large organizations | Sync time proportional to organization size | Honour `Retry-After`; exponential backoff on 429; configure modest per-page `limit` values |
| Workspace iteration slow when many workspaces exist | Linear scaling of workspace_members sync time | Substream pattern with reasonable page size; accept linear scaling |
| `actor_identifier` null for non-user actor types | Identity resolution silently skips these rows | Silver model filters to `actor_type = 'user'`; `api_actor` rows retained in Bronze but not routed for identity resolution |
| Admin API schema change (field added/removed) | Bronze schema may drift | Pin `anthropic-version: 2023-06-01`; schemas include `additionalProperties: true`; monitor Anthropic release notes |
| Migration from legacy `insight_claude_api` / `insight_claude_team` discriminators | Downstream consumers break | No production data yet (per decision Q2 2026); Silver pipeline updated in lockstep with connector swap |
| Invites field rename (`invited_at` → `created_at`) in merged schema | Historical data assumed the old name | No production data yet; Bronze schema emits `created_at` only going forward |

## 13. Open Questions

Open questions carried over from the predecessor connectors, now tracked against this merged connector:

| ID | Summary | Owner | Target |
|----|---------|-------|--------|
| OQ-CADM-1 | Cost allocation to usage rows — keep cost as a separate dimension (status quo) or proportionally allocate to messages_usage rows at Silver level | Data Architecture | Q2 2026 |
| OQ-CADM-2 | Web/mobile Claude usage data — the Admin API only exposes Claude Code; web/mobile activity requires the Enterprise Analytics API (`claude-enterprise` connector) or a future endpoint | Data Architecture | Q2 2026 |
| OQ-CADM-3 | `class_ai_dev_usage` unified schema — Claude Code metrics differ from Cursor/Windsurf; nullable columns per tool vs separate tables per tool category | Data Architecture | Q2 2026 |
| OQ-CADM-4 | Backfill depth — configurable `start_date` (default 90 days ago) is sufficient for most cases; deeper backfill requires explicit override | Connector Team | Q2 2026 |
| OQ-CADM-5 | Cache token billing rates — `cache_creation_5m_tokens` and `cache_creation_1h_tokens` billed differently; Silver aggregates them as `cache_creation_tokens` but Bronze keeps both | Data Architecture | Q2 2026 |

## 14. Non-Applicable Requirements

The following checklist domains have been evaluated and determined not applicable for this connector:

| Domain | Reason |
|--------|--------|
| **Security (SEC)** | Authentication is delegated to the Airbyte framework: the API key is stored as `airbyte_secret`, never logged or exposed. The declarative manifest contains no custom security logic. |
| **Safety (SAFE)** | Pure data-extraction pipeline. No interaction with physical systems. |
| **Performance (PERF)** | Batch connector with native API pagination. Rate-limit handling is the only performance concern, documented in §3.1 and §6.1. |
| **Reliability (REL)** | Idempotent extraction via deduplication keys. Recovery = re-run the sync; the Airbyte framework manages cursor state and retry. |
| **Usability (UX)** | No user-facing interface. Configuration is a K8s Secret. |
| **Compliance (COMPL)** | Work emails are personal data under GDPR. Retention, deletion, and access controls are delegated to the destination operator. The connector must not store credentials outside the platform's secret management. |
| **Maintainability (MAINT)** | Declarative YAML manifest — no custom code to maintain. Schema changes are handled by updating field definitions in the manifest. |
| **Testing (TEST) — custom test tooling only** | Acceptance criteria (TEST-PRD-001) are in §9 with MUST/MUST NOT language (TEST-PRD-002). Airbyte framework checks plus §9 acceptance criteria are sufficient. |
| **Operations — deployment (OPS-PRD-001)** | Deployment is delegated to the orchestrator (Argo Workflows) per the Insight platform model. |
| **Operations — monitoring (OPS-PRD-002)** | Connector-level monitoring is delegated to the orchestrator (see §5.5 Phase 1 deferral note). |
| **Integration — API requirements as producer (INT-PRD-002)** | The connector exposes no API. It is a pure data-extraction consumer. |
