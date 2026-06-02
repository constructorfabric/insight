# PRD — Jira Connector

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
  - [5.1 Issue Data Extraction](#51-issue-data-extraction)
  - [5.2 Activity and Collaboration Extraction](#52-activity-and-collaboration-extraction)
  - [5.3 Directory and Reference Data](#53-directory-and-reference-data)
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
  - [UC-001 Configure Jira Connection](#uc-001-configure-jira-connection)
  - [UC-002 Incremental Sync Run](#uc-002-incremental-sync-run)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Resolved Questions](#13-resolved-questions)
  - [13.1 Phase 1 Scope](#131-phase-1-scope)
- [14. Non-Applicable Requirements](#14-non-applicable-requirements)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Jira Connector extracts issue data, field change history, worklogs, comments, sprint metadata, project directory, issue links, custom fields, and user directory from the Jira REST API and loads them into the Insight platform's Bronze layer. It provides the raw material for measuring developer productivity — cycle time, throughput, work-in-progress, sprint velocity, worklog hours, and blocker analysis — alongside the existing YouTrack connector in a unified task-tracking analytics domain.

### 1.2 Background / Problem Statement

Jira is one of the most widely used task-tracking tools in software organizations. Insight already supports YouTrack as a task-tracking source, but many teams use Jira (Cloud, Server, or Data Center). To deliver unified productivity analytics across the organization, Insight must ingest Jira data into the same Bronze-to-Silver pipeline that already serves YouTrack.

The Jira REST API provides rich data across multiple endpoint groups: issue search with JQL, per-issue changelog, worklogs, comments, project metadata, and the Jira Software Agile API for boards and sprints. The connector must handle the differences between Jira Cloud and Server/Data Center APIs (v3 vs v2), Classic vs Next-gen project models (which affect custom field IDs for story points and sprint assignment), and Atlassian privacy controls that may suppress user email addresses.

The key analytics challenge is that Jira issues are mutable — status, assignee, priority, and sprint assignment change over time. The changelog (field change history) is the source of truth for cycle time and status period calculations, not the current issue snapshot. The connector must capture the complete changelog alongside the issue itself.

**Target Users**:

- Platform operators who configure Jira credentials, project filters, and monitor extraction runs
- Data analysts who consume Jira activity data in Silver/Gold layers alongside YouTrack for unified productivity metrics
- Engineering managers who use cycle time, throughput, sprint velocity, and worklog data for team performance analysis

**Key Problems Solved**:

- Lack of Jira data in the Insight platform, preventing unified task-tracking analytics across Jira and YouTrack teams
- Inability to compute cycle time and status periods without complete field change history
- Missing worklog and comment data needed for effort analysis and collaboration measurement
- No cross-system identity resolution between Jira users and other Insight sources (GitHub, M365, Slack)
- Sprint and issue-link data required for velocity calculations and blocker analysis not available from issue snapshots alone

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- Jira issue data and changelog extracted with no missed sync windows over a 90-day period (Baseline: no Jira extraction; Target: v1.0)
- Per-user Jira activity available for identity resolution within 24 hours of extraction (Baseline: N/A; Target: v1.0)
- Jira data unified with YouTrack in the `task_tracker_*` Silver tables for cross-source analytics (Baseline: YouTrack only; Target: v1.0)

**Capabilities**:

- Extract Jira issues with core fields, custom fields, and complete field change history
- Extract worklogs, comments, sprint metadata, project directory, and issue links
- Incremental extraction using `updated` timestamp as cursor for issues and changelogs
- Identity resolution via `email` from Jira user directory, with platform-specific user ID as fallback (`accountId` on Cloud, `key` on Server/Data Center)
- Support for both Jira Cloud (API v3) and Jira Server/Data Center (API v2) with environment-specific identity anchors

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Jira REST API | Atlassian's REST API for accessing Jira data. Cloud uses v3 (`/rest/api/3/`), Server/Data Center uses v2 (`/rest/api/2/`). |
| Jira Software Agile API | Separate API (`/rest/agile/1.0/`) for boards, sprints, and agile-specific data. |
| Changelog | Per-issue record of every field change (status, assignee, priority, sprint, etc.). Each changelog entry contains one or more field-level changes grouped by a single user action. |
| Worklog | Time logged by a user against a specific issue, recording who worked, when, and for how long. |
| Atlassian Account ID (`accountId`) | Unique, opaque identifier for a user across the Atlassian platform (Jira, Confluence, Bitbucket). Used as internal user key in **Jira Cloud only**. Does not exist in Jira Server/Data Center. |
| User Key (`key`) | Unique, stable identifier for a user in **Jira Server/Data Center**. Not present in Jira Cloud (deprecated and removed). |
| Classic vs Next-gen | Two Jira project models with different custom field IDs for story points and sprint assignment. Classic uses `story_points` or custom fields; Next-gen uses `customfield_10016`. |
| JQL | Jira Query Language — used to filter issues in search requests. |
| Bronze Table | Raw data table in the destination, preserving source-native field names and types without transformation. |

## 2. Actors

### 2.1 Human Actors

#### Platform Operator

**ID**: `cpt-insightspec-actor-jira-operator`

**Role**: Configures Jira instance credentials (API token + email for Cloud, or Basic Auth for Server/Data Center), selects project filters, and monitors extraction runs.
**Needs**: Ability to configure the connector with Jira credentials, filter by project scope, and verify that data is flowing correctly for all streams.

#### Data Analyst

**ID**: `cpt-insightspec-actor-jira-analyst`

**Role**: Consumes Jira issue, changelog, worklog, and sprint data from Silver/Gold layers to build dashboards for cycle time, throughput, sprint velocity, and effort analysis — alongside YouTrack data in unified task-tracking views.
**Needs**: Complete, gap-free issue history with identity resolution to canonical person IDs for cross-platform aggregation.

### 2.2 System Actors

#### Jira REST API

**ID**: `cpt-insightspec-actor-jira-api`

**Role**: External REST API providing issue search, changelogs, worklogs, comments, projects, sprints, and user data. Enforces rate limits and requires authentication via API token (Cloud) or Basic Auth (Server/Data Center).

#### Identity Manager

**Ref**: `cpt-insightspec-actor-identity-manager`

**Role**: Resolves `email` from Jira Bronze user tables to canonical `person_id` in Silver step 2. Enables cross-system joins (Jira + YouTrack + GitHub + M365 + Slack, etc.). When email is unavailable, the platform-specific user ID (`accountId` on Cloud, `key` on Server/DC) serves as a fallback within the Atlassian ecosystem.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires a Jira account with API access and sufficient permissions to read issues, changelogs, worklogs, comments, projects, sprints, and users across the configured project scope
- Jira Cloud requires an API token paired with a user email; Jira Server/Data Center uses Basic Auth or personal access tokens
- The connector operates as a batch collector using incremental sync based on the `updated` field
- The connector **SHOULD** run at least daily to maintain timely changelog and worklog data for cycle time calculations
- Jira API enforces rate limiting; the connector must handle HTTP 429 responses with retry and backoff
- Story points field ID differs between Classic and Next-gen projects — the connector must detect or be configured with the correct field ID per instance

## 4. Scope

### 4.1 In Scope

- Extraction of Jira issues with core fields and the complete field change changelog
- Extraction of custom field values as key-value pairs
- Extraction of worklogs (time logged per issue per user)
- Extraction of comments (collaboration signal)
- Extraction of sprint metadata from the Jira Software Agile API
- Extraction of project directory with project type and style metadata
- Extraction of issue links (dependencies, blockers, duplicates)
- Extraction of Jira user directory for identity resolution
- Connector execution monitoring via collection runs stream
- Incremental sync using `updated` timestamp as cursor
- Identity resolution via `email` and platform-specific user ID (`accountId` on Cloud, `key` on Server/DC)
- Bronze-layer table schemas for all streams
- Support for both Jira Cloud (API v3) and Jira Server/Data Center (API v2)

### 4.2 Out of Scope

- Silver/Gold layer transformations — responsibility of the task-tracking domain pipeline
- Silver step 2 (identity resolution: `email` → `person_id`) — responsibility of the Identity Manager
- Real-time streaming — this connector operates in batch mode
- Jira Service Management (JSM) specific data (SLA metrics, customer satisfaction)
- Confluence, Bitbucket, or other Atlassian product data
- Issue content extraction beyond metadata (no attachment downloads, no embedded media)
- Jira webhooks or event-driven collection
- Custom field auto-discovery and promotion rules — handled at Silver layer

## 5. Functional Requirements

### 5.1 Issue Data Extraction

#### Extract Issues with Core Fields

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-issue-extraction`

The connector **MUST** extract Jira issues with core fields including: internal issue ID, human-readable key, project key, issue type, reporter, story points, due date, parent issue reference, creation timestamp, last update timestamp, and the full API response as raw JSON for future field discovery.

**Rationale**: Issues are the fundamental entity for task-tracking analytics. Core fields provide the identifiers and context needed for all downstream metrics. The raw API response preserves source-native data that may be needed for Silver layer processing without requiring connector changes.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Extract Complete Changelog

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-changelog-extraction`

The connector **MUST** extract the complete field change history for every collected issue, including: the user who made the change, timestamp, field identifier, field name, previous value, new value, and human-readable representations of both values.

**Rationale**: The changelog is the source of truth for cycle time, status period, and assignee history calculations. Current issue state alone is insufficient — analytics require the full sequence of transitions.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Extract Custom Field Values

- [ ] `p2` - **ID**: `cpt-insightspec-fr-jira-custom-fields`

The connector **MUST** extract per-issue custom field values as key-value pairs, including: custom field ID, display name, value, and value type hint.

**Rationale**: Organization-specific fields (team, squad, domain, customer) are essential for grouping and filtering analytics. A key-value model avoids schema changes when new custom fields are added.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Detect Story Points Field

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-story-points-detection`

The connector **MUST** implement a hybrid strategy for detecting the story points field:

1. For **Next-gen** projects (determined by `project_style`): use `customfield_10016` unconditionally.
2. For **Classic** projects: query the field metadata API (`GET /rest/api/3/field`) and search for a numeric field named "Story Points".
3. If multiple candidate fields are found (common in legacy instances with field sprawl): require the Platform Operator to explicitly select the correct field during connection configuration (UC-001).
4. If no story points field is detected and the operator does not configure one: the connector **MUST** emit `null` for story points and log a warning, rather than failing extraction.

**Rationale**: Story points are a core metric for sprint velocity, but the field ID is not standardized across Jira instances and project types. Silent extraction of the wrong field is worse than a null — it produces misleading velocity data. The hybrid strategy (auto-detect + operator fallback) balances automation with correctness.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-operator`

### 5.2 Activity and Collaboration Extraction

#### Extract Worklogs

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-worklog-extraction`

The connector **MUST** extract worklog entries for collected issues, including: worklog ID, parent issue reference, author, work start date, time spent in seconds, and optional comment.

**Rationale**: Worklogs measure actual effort invested per person per issue. This complements status history — an issue may be "In Progress" for weeks but have only a few hours of logged work.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Extract Comments

- [ ] `p2` - **ID**: `cpt-insightspec-fr-jira-comment-extraction`

The connector **MUST** extract comments for collected issues, including: comment ID, parent issue reference, author, creation timestamp, last edit timestamp, and comment body as plain text.

**Rationale**: Comment volume per person is a collaboration signal used in cross-team communication analysis and review participation metrics.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

### 5.3 Directory and Reference Data

#### Extract Sprint Metadata

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-sprint-extraction`

The connector **MUST** extract sprint metadata from the Jira Software Agile API, including: sprint ID, board ID, board name, sprint name, state, start date, end date, and completion date.

**Rationale**: Sprint metadata is required for sprint velocity calculations and carry-over analysis.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Capture Full Sprint Assignment History

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-sprint-history`

The connector **MUST** capture the full history of issue-to-sprint assignment by extracting changelog entries where `field_name = "Sprint"`. Each sprint change event **MUST** preserve the sprint added to, the sprint removed from, and the timestamp of the transition. The connector **MUST NOT** rely solely on the issue's current sprint field, as this loses carry-over context.

**Rationale**: Productivity analytics require understanding when issues are carried over from one sprint to the next. If only the current sprint assignment is captured, the platform cannot distinguish a task that was delivered on time from one that failed delivery in two previous sprints. Full sprint history enables accurate velocity calculations and carry-over rate metrics.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Extract Project Directory

- [ ] `p2` - **ID**: `cpt-insightspec-fr-jira-project-extraction`

The connector **MUST** extract the Jira project directory, including: project ID, project key, name, lead, project type, project style (Classic/Next-gen), and archived status.

**Rationale**: Project metadata maps issues to teams and departments. The project style field is critical because Classic and Next-gen projects use different custom field IDs for story points and sprint assignment.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Extract Issue Links

- [ ] `p2` - **ID**: `cpt-insightspec-fr-jira-issue-links`

The connector **MUST** extract issue links (dependencies and relationships) for collected issues, including: source issue, target issue, and link type name.

**Rationale**: Issue links enable blocker and dependency analysis. Blocked issues should not count against the assignee's throughput in productivity metrics.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-analyst`

#### Extract User Directory

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-user-extraction`

The connector **MUST** extract the Jira user directory, including: user ID (`accountId` on Cloud, `key` on Server/DC), email (when available), display name, account type (Cloud only), and active status.

**Rationale**: The user directory provides the identity attributes needed to associate changelogs, worklogs, and comments with source users and to support downstream identity resolution via the Identity Manager.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-identity-manager`

### 5.4 Connector Operations

#### Track Collection Runs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-jira-collection-runs`

The connector **MUST** produce a collection run log entry for each execution, recording: run ID, start/end time, status, per-stream record counts, API call count, and error count.

**Rationale**: Operational visibility into connector health. Enables alerting on failed runs and tracking data completeness over time.

**Actors**: `cpt-insightspec-actor-jira-operator`

### 5.5 Data Integrity

#### Deduplicate by Primary Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-deduplication`

Each stream **MUST** define a primary key that ensures re-running the connector for an overlapping date range does not produce duplicate records.

**Rationale**: The incremental sync window may overlap with previously fetched dates. Deduplication ensures idempotent extraction.

**Actors**: `cpt-insightspec-actor-jira-api`

#### Support Incremental Collection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-incremental-sync`

The connector **MUST** support incremental collection using the issue `updated` timestamp as cursor, so that ongoing runs process only newly created or modified issues without requiring full reloads. Child streams (changelog, worklogs, comments, links, custom fields) are scoped by the parent issue set.

**Known limitation**: Incremental sync by `updated` is blind to hard-deleted entities (deleted worklogs, deleted issues). Deletion of a child entity (e.g., a worklog) does not reliably update the parent issue's `updated` timestamp in a way that is captured by incremental JQL. The connector **SHOULD** support a periodic full reconciliation run (e.g., weekly) to detect and mark records that no longer exist in the source.

**Rationale**: Full reloads are impractical for large Jira instances with hundreds of thousands of issues. Incremental sync is required for sustainable daily operation. Periodic reconciliation mitigates the risk of stale data from deleted entities without requiring full reloads on every run.

**Actors**: `cpt-insightspec-actor-jira-operator`

#### Handle Changelog Fan-Out Under Bulk Updates

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-changelog-throttling`

The connector **MUST** implement request throttling with exponential backoff and concurrency limits when fetching per-issue changelogs, worklogs, and comments. When a large number of issues are returned by the incremental JQL query (e.g., after a bulk status transition), the connector **MUST NOT** fire all child requests concurrently, and **MUST** respect HTTP 429 responses with retry delays.

**Rationale**: Bulk operations in Jira (e.g., moving 1000 issues to backlog) produce a large delta window where every issue requires separate changelog, worklog, and comment requests. Unthrottled fan-out will trigger Jira API rate limits (HTTP 429) and may cause extraction failures or temporary bans. Aggressive exponential backoff and concurrency limits are required for reliable operation at scale.

**Actors**: `cpt-insightspec-actor-jira-api`, `cpt-insightspec-actor-jira-operator`

### 5.6 Identity Resolution

#### Expose Identity Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-identity-key`

All user-attributed streams **MUST** include a non-null `user_id` field that joins to the user directory. The `user_id` value is environment-specific:

- **Jira Cloud (API v3)**: `user_id` = `accountId` (Atlassian Account ID — immutable, shared across Jira, Confluence, Bitbucket)
- **Jira Server/Data Center (API v2)**: `user_id` = `key` (user key — stable internal identifier; `accountId` does not exist in Server/DC)

The user directory **MUST** include `email` when available from the Jira API. The connector **MUST NOT** skip or fail on users with unavailable email:
- **Jira Cloud**: Atlassian privacy controls suppress email by default for most users. The connector **MUST** emit the user record with `email = null` and a valid `accountId`.
- **Jira Server/DC**: Email is typically available but may be restricted by admin policy. The connector **MUST** emit the user record with `email = null` and a valid `key` when email is unavailable.

The Identity Manager (Silver step 2) resolves identity as follows:
1. If `email` is available: link to the canonical `person_id` via the standard email resolution path.
2. If `email` is unavailable: store the `user_id` as an isolated node in the identity graph. The user appears in analytics as a unique but non-deidentified contributor (e.g., "Jira User {user_id}").
3. If the same `user_id` later becomes linkable (e.g., via Atlassian OAuth login on Cloud, or email becoming available on Server/DC), the Identity Manager retroactively merges the node with the resolved `person_id`, backfilling historical activity attribution.

**Rationale**: Jira Cloud and Server/Data Center use fundamentally different user identity models. Cloud uses `accountId` (opaque, immutable, Atlassian-wide); Server/DC uses `key` (stable, instance-scoped). The connector must abstract this difference behind a unified `user_id` field so that downstream analytics and the Silver layer do not need to branch on deployment type. Email remains the canonical cross-system key but cannot be relied upon as mandatory in either environment.

**Actors**: `cpt-insightspec-actor-identity-manager`

#### Stamp Instance and Tenant Context

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jira-instance-context`

Every record emitted by the connector **MUST** include `insight_source_id` (identifying the specific Jira instance) and `tenant_id` (identifying the Insight tenant). These fields are required for multi-instance disambiguation and tenant isolation.

The connector **MUST** generate a surrogate URN-based primary key for issue records in the format `urn:jira:{tenant_id}:{insight_source_id}:{issue_key}`. The original `insight_source_id`, `tenant_id`, and `issue_key` fields **MUST** be preserved as separate columns for filtering and joins. The URN key eliminates the need for composite key joins in downstream analytics, reducing the risk of join errors in dashboards.

**Rationale**: Multiple Jira instances may feed into the same Bronze store. Without `insight_source_id`, issue keys like `PROJ-123` collide across instances. Forcing analysts to write composite-key JOINs on every query is error-prone. A single URN-based surrogate key provides unambiguous identity while keeping the component fields available for filtering.

**Actors**: `cpt-insightspec-actor-jira-operator`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Data Freshness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-jira-freshness`

The connector **MUST** deliver extracted data to the Bronze layer within 24 hours of the connector's scheduled run.

**Threshold**: Data available in Bronze ≤ 24h after scheduled collection time.

**Rationale**: Timely changelog and worklog data enables near-real-time cycle time and effort dashboards. Stale data reduces the value of sprint retrospective analytics.

#### Extraction Completeness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-jira-completeness`

The connector **MUST** extract all issues matching the configured project scope and date range on each successful run. Failed or partial runs must be detectable and retryable without data loss.

**Rationale**: Incomplete issue extraction leads to incorrect cycle time calculations, understated throughput, and unreliable sprint velocity metrics.

#### Timestamp Normalization

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-jira-utc-timestamps`

All timestamps persisted in the Bronze layer **MUST** be stored in UTC (ISO 8601 format). The connector **MUST** normalize any timezone-aware timestamps from the Jira API to UTC before writing to Bronze.

**Threshold**: Zero non-UTC timestamps in Bronze tables.

**Rationale**: Jira returns timestamps with timezone offsets that reflect the server or user locale. For distributed teams spanning multiple timezones, inconsistent timestamp storage would corrupt cycle time and status period calculations at the Silver/Gold layer. UTC normalization at Bronze guarantees that downstream analytics compute durations correctly regardless of team geography.

### 6.2 NFR Exclusions

- **Real-time streaming latency**: Not applicable — this connector operates in batch mode with daily incremental sync.
- **Throughput / high-volume optimization**: Not applicable for most streams. Large Jira instances may require pagination tuning for issue search but the API handles this natively.
- **Availability**: Batch connector — availability is determined by the orchestrator's scheduling, not by this connector.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Jira Stream Contract

- [ ] `p1` - **ID**: `cpt-insightspec-interface-jira-streams`

**Type**: Data format (Bronze table schemas)

**Stability**: stable

**Description**: Ten Bronze streams with defined schemas — `jira_issue`, `jira_issue_history`, `jira_issue_ext`, `jira_worklogs`, `jira_comments`, `jira_projects`, `jira_issue_links`, `jira_sprints`, `jira_user`, `jira_collection_runs`. All user-attributed streams reference `user_id` as the user key (`accountId` on Cloud, `key` on Server/DC). Issues use `updated` as the cursor field. The URN-based primary key for issue records follows the format `urn:jira:{tenant_id}:{insight_source_id}:{issue_key}` (see FR `cpt-insightspec-fr-jira-instance-context`).

**Field-level schemas**: Defined in [`jira.md`](../jira.md) (Bronze table definitions with column types, descriptions, and API field mappings).

**Bronze-to-Silver mapping**: Defined in the [Task Tracking unified schema](../README.md) (Section "Jira" — mapping from `jira_*` Bronze tables to `task_tracker_*` Silver tables).

**Breaking Change Policy**: Adding new fields is non-breaking. Removing or renaming fields requires a migration.

### 7.2 External Integration Contracts

#### Jira REST API

- [ ] `p1` - **ID**: `cpt-insightspec-contract-jira-rest-api`

**Direction**: required from external system

**Protocol/Format**: REST / JSON

| Stream | Endpoint | Method |
|--------|----------|--------|
| `jira_issue` | `/rest/api/3/search` (JQL with `updated` cursor) | GET — incremental |
| `jira_issue_history` | `/rest/api/3/issue/{key}/changelog` | GET — child of issues |
| `jira_issue_ext` | Custom fields from issue response | Extracted from issue payload |
| `jira_worklogs` | `/rest/api/3/issue/{key}/worklog` | GET — child of issues |
| `jira_comments` | `/rest/api/3/issue/{key}/comment` | GET — child of issues |
| `jira_projects` | `/rest/api/3/project` | GET — full refresh |
| `jira_issue_links` | `fields.issuelinks` in issue response | Extracted from issue payload |
| `jira_sprints` | `/rest/agile/1.0/board/{boardId}/sprint` | GET — full refresh |
| `jira_user` | `/rest/api/3/users/search` | GET — full refresh |

**Authentication**: API token + email (Cloud) or Basic Auth (Server/Data Center)

**Compatibility**: Jira REST API v3 (Cloud) / v2 (Server/Data Center). Jira Software Agile API v1. Response format is JSON with pagination. Field additions are non-breaking.

## 8. Use Cases

### UC-001 Configure Jira Connection

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-jira-configure`

**Actor**: `cpt-insightspec-actor-jira-operator`

**Preconditions**:

- Jira instance accessible with valid credentials (API token + email for Cloud, or Basic Auth for Server/Data Center)
- User account has read permissions across the target projects

**Main Flow**:

1. Operator provides Jira instance URL and credentials
2. System validates credentials against the Jira API
3. System discovers available projects and their project styles (Classic/Next-gen)
4. Connector auto-discovers all projects accessible to the token via `GET /rest/api/3/project/search` — no operator input required (see ADR-007)
5. System auto-detects the story points field: Next-gen projects use `customfield_10016`; Classic projects are scanned via the field metadata API
6. If multiple candidate story points fields are found in Classic projects, system presents a selection list to the operator
7. System initializes the connection with empty state

**Postconditions**:

- Connection is ready for first sync run

**Alternative Flows**:

- **Invalid credentials**: System reports authentication failure (HTTP 401); operator corrects credentials
- **Insufficient permissions**: API returns 403 for some projects; system reports which projects are inaccessible
- **Server/Data Center API version**: System detects API v2 and adjusts endpoint paths accordingly
- **Ambiguous story points field**: Multiple candidate fields found in Classic projects; system prompts operator to select the correct field from the list

### UC-002 Incremental Sync Run

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-jira-incremental-sync`

**Actor**: `cpt-insightspec-actor-jira-operator`

**Preconditions**:

- Connection configured and credentials valid
- Previous state available (or empty for first run)

**Main Flow**:

1. Orchestrator triggers the connector with current state
2. Connector queries issues updated since last cursor using JQL
3. For each updated issue: extract core fields, changelog, worklogs, comments, custom fields, and issue links
4. Connector refreshes project directory, sprint metadata, and user directory (full refresh)
5. Updated cursor position captured after successful write
6. Collection run log entry written

**Postconditions**:

- Bronze tables contain new and updated records
- State updated with latest `updated` timestamp
- Collection run log records success/failure and per-stream counts

**Alternative Flows**:

- **First run**: Connector extracts all issues matching the project scope (full initial load)
- **API throttling (HTTP 429)**: Connector retries with backoff
- **Changelog unavailable for issue**: Connector records the limitation and continues with remaining issues
- **Large result set**: Connector paginates through all pages; no truncated results

## 9. Acceptance Criteria

- [ ] Issues extracted from a live Jira instance with core fields and complete changelog
- [ ] Worklogs and comments extracted for collected issues
- [ ] Sprint metadata extracted from Agile boards
- [ ] Project directory extracted with project type and style
- [ ] Issue links extracted for dependency analysis
- [ ] User directory extracted with `user_id` (`accountId` on Cloud, `key` on Server/DC) and `email` (where available)
- [ ] Custom field values extracted as key-value pairs
- [ ] Incremental sync on second run extracts only newly updated issues (no full reload)
- [ ] `user_id` is present and non-null in all user-attributed records
- [ ] `insight_source_id` is present in all records for multi-instance support
- [ ] Collection run log records success, record counts, and timing for each run

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Jira REST API | Issue search, changelog, worklog, comment, project, and user endpoints | `p1` |
| Jira Software Agile API | Board and sprint metadata endpoints | `p1` |
| Jira credentials | API token + email (Cloud) or Basic Auth (Server/Data Center) | `p1` |
| Airbyte Declarative Connector framework | Execution model for running the connector | `p1` |
| Identity Manager | Resolves `email` to `person_id` in Silver step 2 | `p2` |
| Destination store (PostgreSQL / ClickHouse) | Target for Bronze tables | `p1` |

## 11. Assumptions

- The Jira instance is accessible via REST API with sufficient permissions for the configured project scope
- Jira Cloud uses API v3; Jira Server/Data Center uses API v2 with equivalent functionality for the required endpoints
- Jira Cloud uses `accountId` as the stable, non-null, immutable user identifier shared across the Atlassian platform (Jira, Confluence, Bitbucket); Jira Server/Data Center uses `key` as the stable user identifier (instance-scoped, not cross-platform)
- `email` is suppressed by default in Jira Cloud under Atlassian privacy controls and may be restricted by admin policy on Server/DC; the connector treats email as optional and falls back to `user_id`-only identity when email is unavailable
- The changelog API returns the complete field change history for each issue, not a truncated subset
- Story points field ID differs between Classic and Next-gen projects; the connector uses hybrid auto-detection with operator fallback for ambiguous cases (see FR `cpt-insightspec-fr-jira-story-points-detection`)
- Sprint-to-issue assignment is tracked as full history via changelog entries for the `Sprint` field; current-only assignment is insufficient for carry-over analysis (see FR `cpt-insightspec-fr-jira-sprint-history`)
- The Jira Software Agile API is available for instances with Jira Software license
- JQL-based issue search supports `updated` as an incremental sync cursor with reliable ordering
- All timestamps from the Jira API include timezone information sufficient for UTC normalization
- Deletion of worklogs, comments, or issues does not reliably update the parent issue's `updated` field; periodic reconciliation is needed to detect removed entities

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Email unavailable (Cloud: privacy controls; Server/DC: admin policy) | Identity resolution fails for affected users; analytics gaps in cross-system joins | Use platform-specific `user_id` (`accountId` / `key`) as fallback; surface coverage gaps to operators; support deferred merge when email becomes available |
| Jira Server/DC identity model differs from Cloud | `accountId` does not exist in Server/DC; using it unconditionally would break Server/DC support | Abstract behind unified `user_id` field; connector detects API version and maps to `accountId` (Cloud) or `key` (Server/DC) — see FR `cpt-insightspec-fr-jira-identity-key` |
| Story points field ID varies across instances and project types | Story points data missing or extracted from wrong field | Auto-detect via project style metadata or require explicit per-instance configuration |
| Large Jira instances with 100K+ issues | Initial full load takes extended time; risk of timeout or rate limiting | Paginate with appropriate page size; support project-scoped extraction; handle HTTP 429 with backoff |
| Changelog API fan-out per issue (N+1 problem) | Bulk updates (e.g., manager moves 1000 issues to backlog) produce a large delta window; each issue requires separate changelog, worklog, and comment requests, almost guaranteeing HTTP 429 | Implement concurrency limits, exponential backoff, and request batching — see FR `cpt-insightspec-fr-jira-changelog-throttling`; monitor API call counts in collection run logs |
| Deleted entities invisible to incremental sync | Deleted worklogs, comments, or issues do not update the parent issue's `updated` timestamp reliably; incremental sync will never detect the deletion | Implement periodic full reconciliation (e.g., weekly) to detect and mark stale records — see FR `cpt-insightspec-fr-jira-incremental-sync` known limitation |
| Timezone inconsistencies in source timestamps | Jira returns timestamps with server/user locale offsets; distributed teams across timezones will produce incorrect cycle time if not normalized | Normalize all timestamps to UTC at Bronze level — see NFR `cpt-insightspec-nfr-jira-utc-timestamps` |
| Jira Server/Data Center API differences | Some endpoints or fields differ from Cloud v3 | Maintain API version detection; document known differences; test against both environments |
| Sprint API requires board enumeration | Must list all boards to discover sprints; board count can be large | Cache board list; refresh boards less frequently than issues |
| Multi-instance `id_readable` collisions | Issue keys like `PROJ-123` can collide across Jira instances | Require `insight_source_id` in all joins; composite primary key includes instance scope |
| Comment body content may contain sensitive information | Extracted comments may include customer names, internal discussions, or code snippets | Data access controls and retention policies are platform responsibilities; document that comment body is extracted for collaboration analytics, not content archiving |

## 13. Resolved Questions

### 13.1 Phase 1 Scope

Functional requirements, use cases, and acceptance criteria above describe the **full target scope** of the Jira connector (Cloud + Server/Data Center). Phase 1 implements the subset below. Server/Data Center support is deferred to a future iteration.

| Capability | Phase 1 | Future |
|------------|---------|--------|
| Jira Cloud (API v3) | In scope | — |
| Jira Server / Data Center (API v2) | Out of scope | Separate iteration |
| Comment body plain text extraction | Deferred to Silver/dbt layer | Bronze-level extraction if needed |
| Story points field resolution | Deferred to Silver/dbt layer (based on project style) | Bronze-level extraction if needed |

Implementation details and technical limitations are documented in [DESIGN.md](./DESIGN.md) §4 "Phase 1 Limitations and Future Work".

All open questions from the initial draft have been resolved and incorporated into the PRD as concrete requirements:

| ID | Summary | Resolution | Incorporated In |
|----|---------|------------|-----------------|
| OQ-JIRA-1 | `account_id` vs email as primary identity key | Environment-specific `user_id` is the identity anchor: `accountId` on Cloud, `key` on Server/DC. Email resolved when available; unavailable users stored as isolated nodes with deferred retroactive merge. | FR `cpt-insightspec-fr-jira-identity-key` |
| OQ-JIRA-2 | Multi-instance collision prevention | URN-based surrogate key `urn:jira:{tenant_id}:{insight_source_id}:{issue_key}` as PK. Original fields preserved as separate columns for filtering. | FR `cpt-insightspec-fr-jira-instance-context` |
| OQ-JIRA-3 | Story points field detection | Hybrid strategy: Next-gen → `customfield_10016`; Classic → auto-detect via field metadata API; ambiguous → operator selects during configuration. | FR `cpt-insightspec-fr-jira-story-points-detection` |
| OQ-JIRA-4 | Sprint-issue membership history | Full historical: all sprint assignment changes captured via changelog (`field_name = "Sprint"`). Current-only assignment rejected — carry-over analysis requires the complete transition history. | FR `cpt-insightspec-fr-jira-sprint-history` |

## 14. Non-Applicable Requirements

The following checklist domains have been evaluated and determined not applicable for this connector:

| Domain | Reason |
|--------|--------|
| **Security (SEC)** | The connector handles a single API key or token, marked `airbyte_secret` by the Airbyte framework. No custom authentication, authorization, or encryption logic exists in the declarative manifest. Credential storage and secret management are delegated to the Airbyte platform. Work emails extracted into `jira_user` are personal data under GDPR; retention, deletion, and access controls are platform and destination operator responsibilities. |
| **Safety (SAFE)** | Pure data extraction pipeline. No interaction with physical systems, no potential for harm to people, property, or environment. |
| **Performance (PERF)** | Batch connector with native API pagination. No caching, pooling, or latency optimization needed. Rate limit handling (HTTP 429 retry) is the only performance concern, covered in Section 3.1. |
| **Reliability (REL)** | Idempotent extraction via deduplication keys. No distributed state, no transactions, no saga patterns. Recovery is handled by re-running the sync (Airbyte framework manages state). |
| **Usability (UX)** | No user-facing interface. Configuration is a credential form and project filter in the Airbyte UI. No accessibility, internationalization, or inclusivity requirements apply. |
| **Compliance (COMPL)** | Work emails are personal data under GDPR. Jira comment body content may contain sensitive information. Retention, deletion, and data subject rights are delegated to the Airbyte platform and destination operator. The connector must not store credentials outside the platform's secret management. Data residency and access controls are platform responsibilities. |
| **Maintainability (MAINT)** | Declarative YAML manifest — no custom code to maintain. Schema changes are handled by updating field definitions in the manifest. |
| **Testing (TEST)** | Connector behavior must satisfy PRD acceptance criteria (Section 9). Validation includes: Airbyte framework connection check, schema validation, and connector-specific acceptance tests. No custom unit tests required — the declarative manifest is validated by the framework. |
