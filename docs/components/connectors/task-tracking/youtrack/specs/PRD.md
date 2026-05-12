# PRD — YouTrack Connector

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
  - [5.1 Connector Foundation](#51-connector-foundation)
  - [5.2 Directory Stream Extraction](#52-directory-stream-extraction)
  - [5.3 Issue and Substream Extraction](#53-issue-and-substream-extraction)
  - [5.4 Custom Field Registry Extraction](#54-custom-field-registry-extraction)
  - [5.5 Identity Resolution](#55-identity-resolution)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-001 Configure YouTrack Connection](#uc-001-configure-youtrack-connection)
  - [UC-002 Incremental Issue Sync Run](#uc-002-incremental-issue-sync-run)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Out-of-Scope Capabilities (Future Work)](#13-out-of-scope-capabilities-future-work)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The YouTrack Connector extracts issue data, activity events, comments, worklogs, project directory, agile board metadata with sprints, user directory, issue-link type registry, and per-project custom-field definitions from the JetBrains YouTrack REST API and loads them into the Insight platform's Bronze layer. It provides the raw material for measuring developer productivity — cycle time, throughput, sprint velocity, worklog hours, blocker analysis — alongside the existing Jira connector in a unified task-tracking analytics domain.

This PRD covers the **Bronze layer only**: declarative Airbyte manifest, ten REST-API streams, K8s-Secret-based identity, and dbt source declarations. The downstream Silver/Gold transformations (per-source staging projections, the `youtrack-enrich` Rust replay binary, the Argo `tt-enrich-youtrack-run` workflow template, the `silver:class_task_*` plug-in) are documented as out-of-scope here and tracked in DECOMPOSITION §2.5–§2.10 + the future-scope README at `docs/components/connectors/task-tracking/youtrack/specs/README.md`.

### 1.2 Background / Problem Statement

YouTrack is JetBrains' commercial task tracker, widely adopted by JetBrains-toolchain shops. Many Insight tenants run YouTrack as their primary task-tracker and require the same Bronze-to-Silver analytics pipeline that PR #205 delivered for Jira.

The YouTrack REST API differs from Jira's in three semantically-important ways:

1. **Activity model** — YouTrack does not expose a `changelog` per issue; instead, it exposes `/api/issues/{id}/activitiesPage` which paginates **backwards in time** via opaque `afterCursor` / `hasAfter` tokens. Each activity row carries `$type`, `category`, `field`, `added[]`, `removed[]`, `timestamp`, and `author`. Reconstructing the per-field state at any historic point requires backward replay starting from the current snapshot — symmetric to Jira's forward changelog walk but algorithmically inverted.

2. **Custom-field registry** — Custom fields are **project-scoped**, not global. The same logical field (e.g. "Severity") may have different IDs, value bundles, and cardinality flags in different projects. Discovering the registry requires fanning out to `/api/admin/projects/{id}/customFields` per project. Jira's global `/rest/api/3/field` returns one flat list across the instance; YouTrack does not.

3. **Cursor field is epoch milliseconds** — `youtrack_issue.updated` is an integer epoch-ms value, not an ISO-8601 string. The Airbyte declarative cursor must use the native `%ms` token in `cursor_datetime_formats` to parse it, while keeping `%Y-%m-%dT%H:%M:%S` for state values persisted by Airbyte.

The connector must produce a Bronze schema that the future `youtrack-enrich` Rust binary can consume to populate `staging.youtrack__task_field_history` — the input to the source-agnostic `silver.class_task_field_history` union model. Schema and tagging contracts are pre-committed by PR #205 and the `silver/task-tracking/schema.yml` artifact; the YouTrack Bronze layer must respect them so the future Silver plug-in is a structural drop-in.

**Target Users**:

- Platform operators who configure YouTrack credentials and monitor extraction runs
- Data analysts who consume YouTrack activity data in Silver/Gold alongside Jira and Git activity for unified productivity metrics
- Engineering managers who use cycle-time, throughput, sprint-velocity, and worklog data for team performance analysis

**Key Problems Solved**:

- Lack of YouTrack data in the Insight platform, preventing unified task-tracking analytics for JetBrains-toolchain customers
- Inability to compute cycle time and status periods without complete per-issue activity history (resolved via `activitiesPage` ingestion at Bronze; replay is future scope)
- Missing worklog and comment data needed for effort analysis and collaboration measurement
- No cross-system identity resolution between YouTrack users and other Insight sources (GitHub, M365, Slack)
- Project-scoped custom field definitions needed for `class_task_field_metadata` not available from issue snapshots alone

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- YouTrack issue data and activity history extracted with no missed sync windows over a 90-day period (Baseline: no YouTrack extraction; Target: v1.0)
- Bronze tables for all 10 streams populated and queryable in ClickHouse within 24 hours of connector deployment (Baseline: N/A; Target: v1.0)
- YouTrack Bronze schema accepted by the silver-package contract (`silver:class_task_*` tags) without any change to the union models delivered by PR #205 (Baseline: N/A; Target: v1.0)

**Capabilities**:

- Extract YouTrack issues with core fields, custom-field values, and embedded link references via incremental sync on the `updated` cursor
- Extract per-issue activity history, comments, and worklogs as substreams of issues, with `incremental_dependency: true` so only issues updated since the last sync re-hit their children
- Extract project directory, user directory, agile boards (with nested sprints), and issue-link types as full-refresh streams
- Extract the per-project custom-field registry (field definitions, bundle values, cardinality flags) as a substream of projects
- Identity resolution via `email` from the YouTrack user directory, with `login`/`id` as fallback when email is suppressed by Hub privacy settings

### 1.4 Glossary

| Term | Definition |
|------|------------|
| YouTrack REST API | JetBrains' REST API for accessing YouTrack data. Single API surface at `/api/`; no Cloud/Server split. |
| `activitiesPage` | Per-issue endpoint exposing the full activity timeline (one row per field change, comment, attachment, link, etc.). Paginated **backwards** via `afterCursor` / `hasAfter`. The analog of Jira's `changelog`, but on a different shape. |
| Activity category | YouTrack tag for an activity item's domain — `CustomFieldCategory`, `SummaryCategory`, `CommentsCategory`, `IssueResolvedCategory`, `LinksCategory`, `TimeTrackingCategory`, etc. ~23 categories enumerated in legacy donor code. |
| `$type` (activity) | Polymorphic discriminator on activity rows — `CustomFieldActivityItem`, `SingleValueActivityItem`, `MultiValueActivityItem`, `TextActivityItem`, etc. Maps to the `EventKind` column in the silver union. |
| Project-scoped custom field | A custom field whose ID, value bundle, and cardinality belong to a single project, not the YouTrack instance. The same logical field name may have different IDs in different projects. |
| Permanent token | YouTrack's bearer authentication primitive — a long-lived token tied to a user account. Generated in YouTrack profile UI. Replaces both Basic Auth and OAuth in this connector. |
| Hub privacy | YouTrack Hub user setting that suppresses email exposure in `/api/users` responses. When set, the API returns `email: null`; the connector must fall back to `login`/`id` for identity. |
| Bronze Table | Raw data table in ClickHouse, preserving source-native field names and types without transformation. |
| Silver staging projection | Per-source dbt model tagged `silver:class_task_*` that reshapes Bronze rows into the columns the union expects. Future scope. |

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-insightspec-actor-youtrack-operator`

**Role**: Configures YouTrack instance credentials (base URL + permanent token), applies the K8s Secret, and monitors extraction runs.

**Needs**: Ability to configure the connector with credentials and verify that data is flowing correctly for all ten streams without per-project allowlisting.

#### Data Analyst

**ID**: `cpt-insightspec-actor-youtrack-analyst`

**Role**: Consumes YouTrack issue, activity, worklog, and sprint data from Bronze (and from future Silver/Gold layers) to build dashboards for cycle time, throughput, sprint velocity, and effort analysis — alongside Jira data in unified task-tracking views.

**Needs**: Complete, gap-free issue and activity history with identity resolution to canonical person IDs for cross-platform aggregation.

### 2.2 System Actors

#### YouTrack REST API

**ID**: `cpt-insightspec-actor-youtrack-api`

**Role**: External REST API providing project, user, issue, activity, comment, worklog, agile board, sprint, issue-link-type, and project-scoped custom-field data. Enforces rate limits and requires Bearer authentication via permanent token.

#### Identity Manager

**Ref**: `cpt-insightspec-actor-identity-manager`

**Role**: Resolves `email` (or `login`/`id` fallback) from the YouTrack Bronze user table to canonical `person_id` during Silver step 2 (future scope). Enables cross-system joins (YouTrack + Jira + GitHub + M365 + Slack).

#### Airbyte Source Definition

**ID**: `cpt-insightspec-actor-youtrack-airbyte-source`

**Role**: Airbyte source runtime that loads the declarative manifest (`connector.yaml`) and executes `check` / `discover` / `read` commands against the YouTrack API. The connector ships as a declarative manifest (no custom Python code) consumed by `airbyte/source-declarative-manifest`.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires a YouTrack account with permanent-token access and sufficient permissions to read projects, users, issues, activities, comments, worklogs, agile boards, sprints, issue-link types, and project-scoped custom fields across the entire instance (no project whitelist — see Connector ADR-003).
- YouTrack Cloud rate-limits aggressive paginated traversal of `activitiesPage` for very active projects; the connector must honour `Retry-After` headers on 429 responses and retry 503 with exponential backoff.
- The connector operates as a batch collector using incremental sync on the `updated` cursor for `youtrack_issue` and `incremental_dependency: true` for its three substreams (`youtrack_issue_history`, `youtrack_comments`, `youtrack_worklogs`).
- The connector **SHOULD** run at least daily — the project-default schedule `0 3 * * *` aligns with the Jira connector's daily cadence and is set in `descriptor.yaml`.
- The connector targets a single tenant per source definition (multi-tenant YouTrack is not supported; one source = one YouTrack base URL + one permanent token).
- `youtrack_issue.updated` is epoch milliseconds; the Airbyte cursor must use `%ms` (not ISO) for `cursor_datetime_formats` to parse record values.

## 4. Scope

### 4.1 In Scope

- Extraction of YouTrack projects, users, agile boards (with nested sprints), and issue-link types as full-refresh streams
- Extraction of YouTrack issues with core fields, custom-field values, and embedded link references as an incremental stream on `updated`
- Extraction of per-issue activity history from `activitiesPage` with cursor pagination as a substream of issues
- Extraction of per-issue comments and worklogs as substreams of issues
- Extraction of project-scoped custom-field registry from `/api/admin/projects/{id}/customFields` as a substream of projects
- `incremental_dependency: true` semantics on every issue-substream so only issues updated since the last sync re-hit their children
- Identity stamping — every emitted Bronze row receives `tenant_id` and `source_id` injected from the K8s Secret via the manifest's `AddFields` transformation
- Bronze-layer ClickHouse tables with `ReplacingMergeTree(_version)` engine and `unique_key` ordering, declared via dbt `bronze_youtrack` source block
- Airbyte Builder-UI compatibility for the manifest (no whole-object `$ref`, native epoch-ms cursor, etc.) — see Connector ADRs

### 4.2 Out of Scope

- **Silver/Gold layer transformations** — including the `youtrack-enrich` Rust binary (replay engine, ClickHouse I/O), the Argo `tt-enrich-youtrack-run` workflow template, per-source dbt staging projections tagged `silver:class_task_*`, and the silver plug-in verification. All deferred to DECOMPOSITION §2.5–§2.10. See `docs/components/connectors/task-tracking/youtrack/specs/README.md` for a status snapshot.
- Silver step 2 (identity resolution: `email` → `person_id`) — responsibility of the Identity Manager
- Real-time streaming — this connector operates in batch mode
- YouTrack Hub / Helpdesk-specific data
- Confluence-equivalent or VCS-integration data (YouTrack VCS commits are not extracted)
- Issue content extraction beyond metadata (no attachment downloads, no embedded media)
- YouTrack webhooks or event-driven collection
- Per-project allowlisting via K8s Secret — the connector ingests every project the token can reach (Connector ADR-003)
- Synthetic `youtrack_issue_links` Bronze stream — issue-link rows are kept inside `youtrack_issue.links[]` and projected during Silver staging (future scope)

## 5. Functional Requirements

### 5.1 Connector Foundation

#### Scaffold Declarative Airbyte Source Package

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-bronze-scaffold`

The connector **MUST** ship as a declarative Airbyte source package at `src/ingestion/connectors/task-tracking/youtrack/`, containing `connector.yaml` (the manifest), `descriptor.yaml` (the Insight ingestion descriptor with `namespace: bronze_youtrack` and `schedule: "0 3 * * *"`), `dbt/schema.yml` (Bronze source declarations consumed by the dbt graph), `README.md` (operator documentation), and a K8s Secret example at `src/ingestion/secrets/connectors/youtrack.yaml.example`.

**Rationale**: The declarative source pattern (no custom Python CDK) is the project's default for HTTP-API connectors. It enables Builder-UI editing, deterministic CI validation via `source.sh validate-strict`, and predictable runtime behaviour through `airbyte/source-declarative-manifest`.

**Actors**: `cpt-insightspec-actor-youtrack-airbyte-source`, `cpt-insightspec-actor-youtrack-operator`

#### Bearer Permanent-Token Authentication

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-bronze-auth-bearer`

The connector **MUST** authenticate to YouTrack using a Bearer permanent token sourced from the K8s Secret field `youtrack_token`. The token is injected via `BearerAuthenticator` in `connector.yaml` and **MUST NOT** be logged in plaintext anywhere in the Airbyte trace output.

**Rationale**: Permanent tokens are the project's chosen YouTrack authentication primitive — long-lived, scoped to a service account, and revocable without rotating user credentials.

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-operator`

#### Retry and Error-Handling Policy

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-bronze-retry-policy`

The connector **MUST** define an HTTP error-handler in the manifest that:

- Retries `429` with `Retry-After` header honoured
- Retries `503` with exponential backoff
- Fails fast on `401` / `403` (invalid token / insufficient permission)
- Treats `404` on substream URLs as soft-fail (drop the partition, do not fail the stream — the parent issue may have been deleted between issue fetch and substream traversal)

**Rationale**: YouTrack Cloud throttles aggressive traversal of `activitiesPage` for very active projects; the retry policy avoids unnecessary sync failures while still surfacing real auth/permission errors.

**Actors**: `cpt-insightspec-actor-youtrack-api`

#### Identity Stamping via `AddFields`

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-identity-stamping`

Every emitted Bronze record **MUST** carry `tenant_id` and `source_id` columns injected via the manifest's `AddFields` transformation, sourced from K8s Secret config keys `insight_tenant_id` and `insight_source_id`. Every `AddFields.fields[]` item **MUST** declare `type: AddedFieldDefinition` (Builder-UI compatibility).

**Rationale**: Multi-tenant Bronze tables require row-level tenant attribution. Injecting at the source rather than during dbt staging avoids ambiguity when the same physical table holds data from multiple tenants.

**Actors**: `cpt-insightspec-actor-youtrack-airbyte-source`

### 5.2 Directory Stream Extraction

#### Extract Project Directory

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-projects`

The connector **MUST** extract YouTrack projects from `GET /api/admin/projects?fields=id,shortName,name,description,archived` via offset pagination (`$skip`/`$top`) as a **full-refresh** stream named `youtrack_projects`.

**Rationale**: The project directory is small, mutable, and consumed by both the custom-field substream (parent) and downstream Silver projections. Full-refresh / overwrite is cheaper than incremental given the cardinality.

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-analyst`

#### Extract User Directory

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-users`

The connector **MUST** extract YouTrack users from `GET /api/users?fields=id,login,fullName,email,banned,guest,avatarUrl` via offset pagination as a **full-refresh** stream named `youtrack_user` (singular noun matches Jira's parallel `jira_user`).

**Rationale**: The user directory anchors identity resolution. Email may be `null` for Hub-privacy-suppressed accounts — fallback to `login`/`id` is enforced at the Silver layer (future scope).

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-identity-manager`

#### Extract Agile Boards and Sprints

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-agiles-sprints`

The connector **MUST** extract agile boards from `GET /api/agiles?fields=id,name,projects(id,shortName)` as the full-refresh stream `youtrack_agiles`, and the nested sprint records as a substream `youtrack_sprints` of each agile board with `fields=sprints(id,name,start,finish,archived,goal)`. Each emitted sprint row **MUST** carry the parent `agile_id` for downstream Silver join logic.

**Rationale**: Sprint metadata is required for sprint velocity and carry-over analysis. Modelling sprints as a board-scoped substream matches YouTrack's API shape and avoids the round-trip explosion of one request per sprint.

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-analyst`

#### Extract Issue-Link Type Registry

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-link-types`

The connector **MUST** extract issue link types from `GET /api/issueLinkTypes?fields=id,name,sourceToTarget,targetToSource,directed,aggregation,readOnly` via offset pagination as a **full-refresh** stream named `youtrack_issue_link_types`.

**Rationale**: Link-type metadata (e.g. `depends on` / `is blocked by` direction labels) is needed to interpret per-issue link rows during Silver staging.

**Actors**: `cpt-insightspec-actor-youtrack-api`

> **NFR**: Directory streams **MUST** declare `sync_mode: full_refresh` and `destination_sync_mode: overwrite` — see [`cpt-insightspec-nfr-youtrack-directory-overwrite`](#directory-overwrite-semantics) in §6.1.

### 5.3 Issue and Substream Extraction

#### Incremental Issue Stream

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-issue-incremental`

The connector **MUST** extract YouTrack issues from `GET /api/issues?query=updated:{from}..{to} order by: updated asc&fields={ISSUE_FIELDS}` as an **incremental** stream named `youtrack_issue` cursor-fielded on `updated`. The cursor **MUST** parse epoch-ms record values via the native `%ms` token in `cursor_datetime_formats`, while accepting `%Y-%m-%dT%H:%M:%S` for state values persisted by Airbyte. The `DatetimeBasedCursor` **MUST** declare `cursor_granularity: PT1S` alongside `step: P30D` (CDK requirement).

**Rationale**: `updated` is YouTrack's universal mutation timestamp. The `%ms`-aware cursor avoids the `format_datetime()` transformation pitfall (literal Jinja template ends up as record value — a runtime-only failure mode not caught by `validate` or `validate-strict`).

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-analyst`

#### Activity History via `activitiesPage` Cursor Pagination

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-activities-cursor`

The connector **MUST** extract per-issue activity history from `GET /api/issues/{youtrack_id}/activitiesPage?fields={ACTIVITIES_FIELDS}&$top=200&categories={ACTIVITY_CATEGORIES}&reverse=true` as a substream of `youtrack_issue` named `youtrack_issue_history`. Pagination **MUST** use `CursorPagination` (`afterCursor` / `hasAfter`) — not offset — to walk activities backwards in time. The `categories` query parameter **MUST** enumerate the 23 activity categories required for replay (see Connector ADR-002 for the canonical list).

**Rationale**: `activitiesPage` is the only YouTrack endpoint that returns activity events with stable ordering across page boundaries. Offset pagination is not supported; cursor pagination is mandatory. The categories whitelist filters out chatter (e.g. attachment churn) that does not contribute to status / field history.

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-analyst`

#### Substream Routing Via `youtrack_id`

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-substream-routing-youtrack-id`

Every issue substream (`youtrack_issue_history`, `youtrack_comments`, `youtrack_worklogs`) **MUST** route its URL path via `stream_partition.youtrack_id` (derived from `record['id']`, always present), not via `id_readable` (derived from `record.get('idReadable')`, nullable). The Bronze table column **MUST** be named `youtrack_id` so the downstream dbt joins back to `youtrack_issue.youtrack_id` for the human-readable identifier.

**Rationale**: Routing via a nullable column silently produces URLs like `/api/issues/None/activitiesPage` which 404 and drop entire issue partitions. The internal `id` is unconditionally present and accepted by every YouTrack issue sub-resource path.

**Actors**: `cpt-insightspec-actor-youtrack-api`

#### Extract Comments

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-comments`

The connector **MUST** extract per-issue comments from `GET /api/issues/{youtrack_id}/comments?fields=id,text,textPreview,created,updated,author(...),deleted,visibility(...)` as a substream of `youtrack_issue` named `youtrack_comments`, with offset pagination.

**Rationale**: Comment volume per person is a collaboration signal used in cross-team communication analysis and review-participation metrics.

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-analyst`

#### Extract Worklogs

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-worklogs`

The connector **MUST** extract per-issue worklogs from `GET /api/issues/{youtrack_id}/timeTracking/workItems?fields=...` as a substream of `youtrack_issue` named `youtrack_worklogs`, with offset pagination.

**Rationale**: Worklogs measure actual effort invested per person per issue. This complements status history — an issue may be "In Progress" for weeks but have only a few hours of logged work.

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-analyst`

#### Embedded Issue Links (Deferred)

- [ ] `p2` - **ID**: `cpt-insightspec-fr-youtrack-stream-issue-links`

YouTrack issue-link rows are embedded in `youtrack_issue.links[]` (no separate REST endpoint). The connector **MUST** preserve this column as raw JSON for downstream projection into a `youtrack_issue_links` table during Silver staging (future scope — DECOMPOSITION §2.5). No separate Bronze stream is emitted for issue links in this PRD's scope.

**Rationale**: A synthetic `RecordSelector`-based stream would double-traverse the issue blob; flattening at dbt staging is cheaper and more uniform with the Jira approach (which also derives `jira__changelog_items` from blob columns).

**Actors**: `cpt-insightspec-actor-youtrack-analyst`

#### Substream Incremental Dependency

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-incremental-dependency`

Every issue substream (`youtrack_issue_history`, `youtrack_comments`, `youtrack_worklogs`) **MUST** declare `incremental_dependency: true` against its `youtrack_issue` parent. Only issues whose `updated` cursor advanced since the last sync re-hit their substream URLs.

**Rationale**: Without `incremental_dependency`, every sync re-walks every issue's full activity / comment / worklog list — a cost proportional to issue count × activity count. For active YouTrack tenants this is prohibitive (the verification table in the PR description shows 6 486 issues × 55 310 activities).

**Actors**: `cpt-insightspec-actor-youtrack-api`

> **NFR**: Bronze ClickHouse tables for issue-scope streams **MUST** use `engine: ReplacingMergeTree` with `order_by: [unique_key]` — see [`cpt-insightspec-nfr-youtrack-issue-replacingmergetree`](#issue-replacingmergetree-semantics) in §6.1.

### 5.4 Custom Field Registry Extraction

#### Extract Per-Project Custom Field Definitions

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-stream-project-custom-fields`

The connector **MUST** extract per-project custom field definitions from `GET /api/admin/projects/{id}/customFields?fields=id,field(id,name,localizedName,fieldType(id,valueType,isMultiValue)),bundle(id,values(id,name,description,archived,color(id,presentation),ordinal)),canBeEmpty,ordinal,emptyFieldText,isPublic` as a substream of `youtrack_projects` named `youtrack_project_custom_fields`, with offset pagination. The parent `project_id` **MUST** be injected into every emitted row.

**Rationale**: Custom fields are project-scoped in YouTrack — the same logical field has different IDs and bundle values across projects (Connector ADR-001). Discovering the registry per project is the only correct shape and feeds `class_task_field_metadata` (future scope).

**Actors**: `cpt-insightspec-actor-youtrack-api`, `cpt-insightspec-actor-youtrack-analyst`

#### Bundle Values Captured Inline

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-custom-field-bundles`

The connector **MUST** include the `bundle.values[]` array in the `youtrack_project_custom_fields` payload, capturing each bundle value's id, name, description, archived flag, colour, and ordinal. A separate top-level `customFieldSettings/bundles` traversal **MUST NOT** be required for the common case.

**Rationale**: Phase 1 research against YouTrack Cloud and Server confirms `/api/admin/projects/{id}/customFields?fields=bundle(values(...))` returns inlined bundle values for the default field types (Enum, OwnedField, Version). Re-walking the global bundle registry would inflate API cost by an order of magnitude for no incremental data.

**Actors**: `cpt-insightspec-actor-youtrack-api`

### 5.5 Identity Resolution

#### Identity By Email With Login Fallback

- [ ] `p1` - **ID**: `cpt-insightspec-fr-youtrack-identity-by-email`

The connector **MUST** capture `email`, `login`, and `id` for every YouTrack user. The Silver identity-resolution step (future scope) **MUST** use `email` as the primary join key, with `login` as the secondary anchor when `email` is `null` (Hub privacy), and the YouTrack internal `id` as the last-resort fallback for ecosystem joins.

**Rationale**: YouTrack Hub allows users to mask their email from API responses. The fallback chain ensures identity continuity at the cost of one additional resolution step in the Silver layer.

**Actors**: `cpt-insightspec-actor-identity-manager`, `cpt-insightspec-actor-youtrack-api`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Directory Overwrite Semantics

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-youtrack-directory-overwrite`

Directory streams (`youtrack_projects`, `youtrack_user`, `youtrack_agiles`, `youtrack_sprints`, `youtrack_issue_link_types`) **MUST** declare `sync_mode: full_refresh` and `destination_sync_mode: overwrite`. The Bronze ClickHouse table is rebuilt every sync; no historical retention is required for reference data.

**Threshold**: Stream size **MUST NOT** exceed 100 000 rows per directory (current YouTrack-Cloud upper bound across observed tenants).

**Rationale**: These streams have low cardinality and rapidly-changing metadata (project archival, sprint renaming). Overwrite avoids retention gymnastics and ensures the latest snapshot is always authoritative.

#### Issue ReplacingMergeTree Semantics

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-youtrack-issue-replacingmergetree`

The Bronze ClickHouse tables for `youtrack_issue`, `youtrack_issue_history`, `youtrack_comments`, and `youtrack_worklogs` **MUST** use `engine: ReplacingMergeTree` with `order_by: [unique_key]` to support idempotent re-sync of unchanged rows. The `unique_key` formula per row **MUST** follow the project-wide bronze convention documented in `docs/domain/ingestion-data-flow/specs/`.

**Threshold**: Table sizes **MUST NOT** exceed 100 M rows per stream within the first six months of operation under normal sync cadence.

**Rationale**: Backward-replay (future) re-emits historical rows on every sync; without dedup, the table grows monotonically. `ReplacingMergeTree(_version)` keeps the latest by `_version` per unique key.

#### Secret Rotation Without Connector Restart

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-youtrack-secret-rotation`

The connector **MUST** read the YouTrack permanent token from a K8s Secret mounted as an Airbyte source config at sync-start time. Operators rotating the token **MUST NOT** need to restart, redeploy, or re-register the Airbyte source — applying the new Secret value and triggering a fresh sync is sufficient.

**Threshold**: Token rotation latency from `kubectl apply -f secret.yaml` to the next sync picking up the new token **MUST** be ≤ 24 hours.

**Rationale**: Operators rotate tokens on routine compliance schedules. Forcing connector redeploy would either delay rotation or risk skipped sync windows.

#### No Plaintext Token in Logs

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-youtrack-no-log-token`

The connector **MUST NOT** emit the permanent token (or any K8s Secret value) in plaintext to stdout, stderr, Airbyte trace logs, or any error message. Token redaction is delegated to the `airbyte/source-declarative-manifest` runtime's standard secret-handling.

**Verification Method**: Inspection of `source.sh read` output for any secret string after a deliberate-failure sync.

#### Idempotent Re-Sync

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-youtrack-idempotency`

A second sync executed without source data changes **MUST** produce identical Bronze row counts as the first sync (modulo `_airbyte_emitted_at` per-row timestamps). The `unique_key`-keyed `ReplacingMergeTree` engine collapses duplicate emissions automatically.

**Threshold**: Bronze table row count delta between two consecutive no-change syncs ≤ 0.01% (allowing for legitimate concurrent edits during the sync window).

**Verification Method**: E2E smoke run from DECOMPOSITION §2.10 (future scope) — compare counts after two consecutive `run-sync.sh youtrack <tenant>` invocations.

#### Schema-Drift Detection

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-youtrack-schema-drift-detection`

The connector schema (declared by `InlineSchemaLoader` in `connector.yaml` and registered in `dbt/schema.yml`) **MUST** be regenerated and committed when the YouTrack API adds new top-level fields. `generate-schema.sh youtrack` and `dbt parse` (CI gate) detect drift on every PR.

### 6.2 NFR Exclusions

The following project-default NFRs do not apply to this Bronze-only connector:

- **Real-time latency thresholds** — not applicable; this connector is daily-batch by design (PRD §3.1).
- **Bronze-layer schema validation** — Bronze ingests source-native shapes; schema validation happens at Silver staging (future scope §2.5).
- **Cross-source dedup invariants** — owned by the Silver layer; not applicable at Bronze.
- **Sub-hour rotation latency** — covered by the 24-hour rotation NFR (`cpt-insightspec-nfr-youtrack-secret-rotation`); finer-grained rotation is explicitly out of scope.

## 7. Public Library Interfaces

### 7.1 Public API Surface

The connector is consumed via two interfaces:

1. **The Insight ingestion pipeline (production)** — `kubectl apply -f src/ingestion/secrets/connectors/youtrack.yaml`, then `run-sync.sh youtrack <tenant>` (Argo `ingestion-pipeline` template branch `youtrack`, future scope §2.8) or the daily cron schedule defined in `descriptor.yaml`.

2. **The local declarative-connector toolkit (development)** — `./tools/declarative-connector/source.sh {validate-strict,validate,check,discover,read} task-tracking/youtrack [<tenant>]` for manifest editing, schema regeneration, and per-stream smoke testing.

### 7.2 External Integration Contracts

| Contract | Counterparty | Shape |
|---|---|---|
| K8s Secret `insight-youtrack-<source-id>` | Platform Operator | `data: { insight_tenant_id, insight_source_id, youtrack_base_url, youtrack_token, youtrack_start_date?, youtrack_page_size?, youtrack_activities_page_size? }` |
| Airbyte source definition | Airbyte runtime | Declarative manifest `connector.yaml` v7.0.4 — `version`, `type: DeclarativeSource`, `check`, `definitions`, `streams[]`, `spec`, `concurrency_level`, `metadata` |
| Bronze ClickHouse tables | dbt + downstream Silver | Source block `bronze_youtrack.{youtrack_projects,youtrack_user,youtrack_agiles,youtrack_sprints,youtrack_issue_link_types,youtrack_issue,youtrack_issue_history,youtrack_comments,youtrack_worklogs,youtrack_project_custom_fields}` |

## 8. Use Cases

### UC-001 Configure YouTrack Connection

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-youtrack-configure-connection`

**Actor**: Platform Operator (`cpt-insightspec-actor-youtrack-operator`)

**Preconditions**:

- Operator has a YouTrack instance URL (e.g. `https://example.youtrack.cloud`)
- Operator has generated a permanent token in YouTrack profile UI with read access to projects, issues, users, agile boards, and project custom fields
- Insight platform deployed with the umbrella chart; `bronze_youtrack` schema exists in ClickHouse

**Main Flow**:

1. Operator copies `src/ingestion/secrets/connectors/youtrack.yaml.example` to a working file
2. Operator fills `youtrack_base_url`, `youtrack_token`, and Insight identity fields (`insight_tenant_id`, `insight_source_id`)
3. Operator optionally overrides `youtrack_start_date`, `youtrack_page_size`, `youtrack_activities_page_size`
4. Operator applies the Secret: `kubectl apply -f youtrack.yaml`
5. Operator validates the connection: `./tools/declarative-connector/source.sh check task-tracking/youtrack <source-id>` — expects `{"status":"SUCCEEDED"}`
6. Operator triggers the first sync: `./src/ingestion/run-sync.sh youtrack <source-id>` (future scope §2.8) — or waits for the daily cron at 03:00 UTC

**Postconditions**:

- Bronze tables populated with first-sync data
- Subsequent syncs are incremental via `updated` cursor on `youtrack_issue` and `incremental_dependency` propagation

**Exceptions**:

- Invalid token → `check` returns `FAILED` with `401 Unauthorized`. Operator regenerates token, updates Secret, repeats step 4.
- Token lacks required permission on a project → `check` succeeds but `read` against `youtrack_project_custom_fields` returns `403` on that project; the error-handler drops the partition and logs a warning.

### UC-002 Incremental Issue Sync Run

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-youtrack-incremental-sync`

**Actor**: Airbyte Source Definition (`cpt-insightspec-actor-youtrack-airbyte-source`)

**Preconditions**:

- UC-001 completed; first sync recorded `state.json` for `youtrack_issue`
- Source API returns at least one issue with `updated > state.cursor_value`

**Main Flow**:

1. Airbyte loads `state.json` and renders the request URL with `updated:{state.cursor_value}..{now} order by: updated asc`
2. Airbyte paginates `youtrack_issue` via `$skip`/`$top` until response is empty
3. For each emitted issue with `updated > state.cursor_value`, the three substreams (`_issue_history`, `_comments`, `_worklogs`) fire with their parent's `youtrack_id`
4. `_issue_history` walks `activitiesPage` backwards via `afterCursor` until `hasAfter: false`
5. Every emitted row is decorated with `tenant_id`, `source_id` via `AddFields`
6. The `STATE` message is updated to the highest `updated` value observed
7. Bronze tables receive the new rows; `ReplacingMergeTree(_version)` collapses any retransmitted historical rows

**Postconditions**:

- `youtrack_issue.updated` cursor advanced to the latest mutation timestamp
- All four issue-scope tables consistent

**Exceptions**:

- `429 Too Many Requests` → retry with `Retry-After` (manifest error-handler)
- `503 Service Unavailable` → exponential backoff retry
- `404 Not Found` on substream URL (issue deleted mid-sync) → drop partition, continue

## 9. Acceptance Criteria

- All ten streams declared in `connector.yaml` pass `source.sh validate-strict` (Builder-UI compatible)
- All ten streams pass `source.sh validate` (CDK runtime)
- `source.sh check` against a live test tenant returns `SUCCEEDED`
- `source.sh discover` reports all ten streams with the expected sync modes (`youtrack_issue` incremental, rest full-refresh)
- Per-stream `source.sh read` succeeds with zero `ERROR` messages for the full set of activity categories enumerated in Connector ADR-002
- The manifest opens cleanly in the Airbyte Builder UI on `cloud.airbyte.com` (manual operator check; not CI-automated)
- `dbt parse` succeeds with the YouTrack `bronze_youtrack` source block registered (CI gate from PR #382)
- DECOMPOSITION traceability validates: every FR/NFR ID in §2.1–§2.4 of `DECOMPOSITION.md` resolves to a real ID in this PRD

## 10. Dependencies

- **PR #205** (merged) — Silver task-tracking package (`silver/task-tracking/class_task_*` union models, `create_task_field_history_staging` dbt macro, `ingestion-pipeline` Argo template). Required by future scope features 2.5–2.10 but **not** by Bronze (this PRD).
- **PR #251** — bronze→staging→silver data-flow conventions. The YouTrack Bronze tables must respect engine + `unique_key` rules.
- **PR #281** — version-driven reconcile loop. `descriptor.yaml` version semantics inherit from this PR.
- **PR #363** — `bronze-promoted` validator. Future scope §2.5 must satisfy this contract (not required at Bronze).
- **PR #382** — `dbt parse` CI gate. `dbt/schema.yml` must parse cleanly.
- Project-wide **declarative-connector** toolkit (`src/ingestion/tools/declarative-connector/`) — `validate-strict`, `validate`, `check`, `discover`, `read`.
- Project-wide **silver task-tracking** package (`src/ingestion/silver/task-tracking/`) — `class_task_*` union models, schema contract.

## 11. Assumptions

- YouTrack Cloud and YouTrack Server expose identical REST shapes for the ten endpoints used by this connector (verified in Phase 1 research)
- Permanent tokens grant identical access scope across `/api/admin/...` and `/api/...` (no hub-only token scope split)
- `activitiesPage` cursor tokens are stable across page boundaries within a sync window — i.e. an `afterCursor` produced at T0 is still resolvable at T0+5min
- The K8s Secret rotation cycle (≤ 24h) is sufficient — operators do not require sub-hour rotation
- Tenant cardinality remains ≤ 100 M rows per Bronze stream over the next six months under daily-sync cadence

## 12. Risks

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| YouTrack changes `activitiesPage` cursor token format | Low | High | Manifest declares `cursor_value: "{{ response.afterCursor }}"` — token shape is opaque to the manifest. Smoke-test in CI on every CDK upgrade. |
| Builder UI rejects the manifest after a CDK upgrade | Medium | Medium | `validate-strict` runs in pre-commit and CI. New CDK versions surface schema drift immediately. |
| Hub privacy suppresses email for the bulk of users | Medium | Medium | Identity fallback chain (email → login → id) defined in §5.5 absorbs this; downstream Silver staging joins on whichever field resolves first. |
| Bronze table row count exceeds 100 M | Low | Medium | `_version` and `unique_key` design supports sharding by tenant + date if needed. Monitor in CI. |
| YouTrack rate-limits aggressive `activitiesPage` traversal under burst load | Medium | Low | `Retry-After` + exponential backoff in the manifest error-handler. `youtrack_activities_page_size` is configurable; operators can lower it. |

## 13. Out-of-Scope Capabilities (Future Work)

The following capabilities are intentionally **out of scope** for this PRD and the PR that delivers it. They are documented in `DECOMPOSITION.md` §2.5–§2.10 and summarized in `docs/components/connectors/task-tracking/youtrack/specs/README.md`. None blocks Bronze acceptance.

- **Feature 2.5** — dbt per-source staging projections tagged `silver:class_task_*` (`youtrack__changelog_items`, `youtrack__issue_field_snapshot`, `youtrack__task_{comments,worklogs,users,projects,sprints,field_metadata,field_history}`)
- **Feature 2.6** — Rust `youtrack-enrich` core (backward replay engine, multi-value semantics, synthetic-initial bootstrap)
- **Feature 2.7** — Rust `youtrack-enrich` IO (ClickHouse reader/writer, schema assert, CLI surface)
- **Feature 2.8** — Argo `tt-enrich-youtrack-run.yaml` workflow template + `ingestion-pipeline` branch
- **Feature 2.9** — Silver plug-in verification (`silver:class_task_*` union rows include YouTrack source)
- **Feature 2.10** — Test invariants + E2E smoke (reuse PR #205 invariants, add YouTrack-specific Rust unit cases)

When this future scope is delivered, the corresponding PRD sections and FR/NFR IDs will be added under §5.6–§5.10 here (incremental amendments, not a rewrite).
