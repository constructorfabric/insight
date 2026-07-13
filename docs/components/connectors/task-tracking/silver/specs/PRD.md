# PRD — Task Tracking Silver Layer

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
  - [5.1 Core Features](#51-core-features)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 Module-Specific NFRs](#61-module-specific-nfrs)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Task Tracking Silver Layer unifies field change history from multiple task tracking systems into a single schema where every record contains the **complete field value** — not a delta. This enables analytics (cycle time, throughput, WIP, sprint velocity) without requiring consumers to reconstruct field state from event-sourced deltas.

### 1.2 Background / Problem Statement

Task tracking systems (YouTrack, Jira, GitHub Projects V2, Azure DevOps) expose field change history as **deltas** — what was added or removed — not as full snapshots. For multi-value fields (labels, sprints, components), a single changelog entry shows only one addition or removal. Reconstructing the complete field value at any point in time requires sequential accumulation of all prior events, which is fragile in SQL and impossible in pure dbt without recursive logic.

Additionally, each source system uses a different format for change history:

- YouTrack: `added[]` / `removed[]` arrays of objects
- Jira: `from` / `to` IDs + `fromString` / `toString` display values
- GitHub Projects V2: no changelog API for project-level fields — only current snapshots
- Azure DevOps: `oldValue` / `newValue` with Tags as full snapshots

Neither YouTrack nor Jira include fields set at issue creation in their changelog — the initial state must be reconstructed from the current issue snapshot.

**Target Users**:

- Data engineers building and maintaining the ingestion pipeline
- Data analysts querying Silver/Gold tables for team productivity metrics
- Engineering managers consuming dashboards derived from Gold metrics

**Key Problems Solved**:

- Consumers cannot query multi-value field history without complex delta accumulation
- No unified schema across task trackers — each has different field types, ID formats, and changelog semantics
- Field type metadata (single vs multi-value) is not available in changelog data and must be obtained separately
- Initial field state at issue creation is absent from changelog in YouTrack and Jira

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- All field changes queryable with full values in a single `SELECT` (Baseline: requires recursive accumulation or Python scripts; Target: v1.0)
- Four task tracker sources unified into one schema (Baseline: 0; Target: YouTrack + Jira + GitHub Projects V2 + Azure DevOps)
- Cycle time, throughput, and sprint velocity derivable from Silver data without additional transformations (Baseline: requires custom scripts; Target: v1.0)

**Capabilities**:

- Query complete field state at any point in issue history
- Analyze field changes across multiple task tracker systems in a single query
- Track field type metadata changes over time
- Support identity resolution for cross-system person attribution

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Field history | A record of every change to a field on an issue, stored with the complete field value after the change |
| Delta | The raw changelog output from a source system showing only what was added or removed |
| Full value | The complete set of values for a field at a given point in time |
| Field cardinality | Whether a field holds a single value or multiple values simultaneously |
| Enrich | The pipeline step that transforms source deltas into full-value records |
| Value ID type | Classification of identifiers: opaque system ID, account ID, string literal, hierarchical path, or none |
| Bronze | Raw data from source APIs, stored as-is |
| Silver | Unified, cleaned, enriched data with full values and identity resolution |
| Gold | Aggregated metrics derived from Silver (cycle time, velocity, throughput) |

## 2. Actors

### 2.1 Human Actors

#### Data Engineer

**ID**: `cpt-insightspec-actor-tt-silver-data-engineer`

**Role**: Configures and maintains the ingestion pipeline (connectors, dbt models, enrich scripts, Argo workflows).

**Needs**: Clear schema definitions, source mapping documentation, field metadata API references, troubleshooting guidance for data quality issues.

#### Data Analyst

**ID**: `cpt-insightspec-actor-tt-silver-data-analyst`

**Role**: Queries Silver and Gold tables to produce team productivity reports and dashboards.

**Needs**: Full-value field history queryable with simple SQL, unified schema across sources, documented field semantics and ID types.

#### Engineering Manager

**ID**: `cpt-insightspec-actor-tt-silver-eng-manager`

**Role**: Consumes dashboards and reports derived from Gold metrics.

**Needs**: Accurate cycle time, sprint velocity, WIP, and workload distribution metrics.

### 2.2 System Actors

#### Enrich Pipeline

**ID**: `cpt-insightspec-actor-tt-silver-enrich-pipeline`

**Role**: Transforms source deltas into full-value records by maintaining running field state. Reads from Bronze/Silver Step 1, writes enriched data to Silver.

**Needs**: Field metadata (single/multi, value ID type), current issue snapshots for initial state reconstruction, ordered changelog events.

#### Identity Manager

**ID**: `cpt-insightspec-actor-tt-silver-identity-manager`

**Role**: Resolves source-specific user IDs to canonical `person_id` via email matching.

**Needs**: `task_tracker_users` table with email addresses, `author_id` references in field history records.

#### Airbyte Connectors

**ID**: `cpt-insightspec-actor-tt-silver-connectors`

**Role**: Extract data from source task tracker APIs and load into Bronze tables.

**Needs**: Connection credentials, API access, sync schedules.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **Storage**: ClickHouse with `ReplacingMergeTree` engine for all Silver tables
- **Orchestration**: Argo Workflows DAG: Airbyte sync -> dbt Step 1 -> Enrich -> dbt Step 2
- **Transformation**: dbt-clickhouse for SQL transformations; Python script for delta-to-full-value enrichment
- **Sources**: YouTrack, Jira Cloud, GitHub Projects V2, Azure DevOps — each with different changelog APIs and field type systems

## 4. Scope

### 4.1 In Scope

- Unified `task_tracker_field_history` table with full field values
- `task_tracker_field_metadata` table for tracking field types over time
- Supporting Silver tables: worklogs, comments, sprints, projects, users, issue links, collection runs
- Source-specific mapping for YouTrack, Jira, GitHub Projects V2, Azure DevOps
- Identity resolution integration (`author_id` -> `person_id`)
- Initial state capture for fields set at issue creation
- Field cardinality (single/multi) and value ID type classification

### 4.2 Out of Scope

- Enrich script implementation details (separate spec)
- Gold layer metric calculations (cycle time, velocity, throughput formulas)
- Connector Bronze layer schemas (defined in per-source specs)
- Dashboard or UI design
- Cross-system field ID normalization (field IDs stored as-is from source)
- Webhook-based real-time sync

## 5. Functional Requirements

### 5.1 Core Features

#### Full-Value Field History

- [ ] `p1` - **ID**: `cpt-insightspec-fr-tt-silver-full-value`

The system **MUST** store the complete field value after every change event, not just the delta.

**Rationale**: Consumers need to query field state at any point in time with a simple `SELECT`, without reconstructing from deltas.

**Actors**: `cpt-insightspec-actor-tt-silver-data-analyst`, `cpt-insightspec-actor-tt-silver-enrich-pipeline`

#### Multi-Source Unification

- [ ] `p1` - **ID**: `cpt-insightspec-fr-tt-silver-multi-source`

The system **MUST** unify field history from YouTrack, Jira, GitHub Projects V2, and Azure DevOps into a single table schema.

**Rationale**: Cross-system analytics require a unified schema. Analysts should not need source-specific queries.

**Actors**: `cpt-insightspec-actor-tt-silver-data-analyst`, `cpt-insightspec-actor-tt-silver-data-engineer`

#### Initial State Capture

- [ ] `p1` - **ID**: `cpt-insightspec-fr-tt-silver-initial-state`

The system **MUST** capture the initial values of all fields populated at issue creation as records in the field history table.

**Rationale**: YouTrack and Jira do not include creation-time field values in their changelog. Without initial state, the first known state of a field is undefined until its first change.

**Actors**: `cpt-insightspec-actor-tt-silver-enrich-pipeline`

#### Field Cardinality Tracking

- [ ] `p1` - **ID**: `cpt-insightspec-fr-tt-silver-cardinality`

The system **MUST** classify each field as single-value or multi-value and store this classification in the field history record.

**Rationale**: Single-value and multi-value fields require different delta accumulation logic. The classification determines how full values are computed from source deltas.

**Actors**: `cpt-insightspec-actor-tt-silver-enrich-pipeline`

#### Field Metadata Collection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-tt-silver-field-metadata`

The system **MUST** collect field type metadata from each source system's API on every sync run and persist it with timestamps.

**Rationale**: Field types can change at runtime (YouTrack allows changing `enum[1]` to `enum[*]` on existing fields). Historical metadata is required to apply correct accumulation logic.

**Actors**: `cpt-insightspec-actor-tt-silver-enrich-pipeline`, `cpt-insightspec-actor-tt-silver-data-engineer`

#### Value ID Type Classification

- [ ] `p2` - **ID**: `cpt-insightspec-fr-tt-silver-value-id-type`

The system **MUST** classify the type of identifier stored in field values: opaque system ID, account ID, string literal, hierarchical path, or none.

**Rationale**: Different ID types require different join and deduplication strategies. Jira labels have no IDs (string literal), while components have opaque IDs. Consumers need this metadata for correct joins.

**Actors**: `cpt-insightspec-actor-tt-silver-data-analyst`

#### Status Category Unification

- [ ] `p1` - **ID**: `cpt-insightspec-fr-tt-silver-status-category`

The system **MUST** expose, per source status, a **source-neutral lifecycle category** (`new` / `in_progress` / `done` / `undefined`) in a status dimension keyed by `status_id`, so consumers can detect a "done"/closed task without matching localized status display names.

**Rationale**: A status *display name* is not a lifecycle signal — it varies by workflow, project template, and locale (`Done`, `Готово`, custom names). Detecting closedness by hardcoded English names (`Closed`/`Resolved`/`Verified`) reports zero completed tasks for default Jira Cloud team-managed projects and every non-English instance (issue #1541). The category signal already exists at the source (Jira `statusCategory`, YouTrack `isResolved`) and must be surfaced through Silver rather than reconstructed downstream.

**Actors**: `cpt-insightspec-actor-tt-silver-data-analyst`, `cpt-insightspec-actor-tt-silver-data-engineer`

#### Delta Preservation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-tt-silver-delta-preservation`

The system **MUST** preserve the original delta information (what was added, removed, or set) alongside the full value in each record.

**Rationale**: Audit trail and debugging require knowing exactly what changed in each event, not just the resulting state.

**Actors**: `cpt-insightspec-actor-tt-silver-data-engineer`, `cpt-insightspec-actor-tt-silver-data-analyst`

#### Identity Resolution Integration

- [ ] `p2` - **ID**: `cpt-insightspec-fr-tt-silver-identity`

The system **MUST** support resolving `author_id` in field history records to canonical `person_id` via the Identity Manager.

**Rationale**: Cross-system person attribution (same person using YouTrack and Jira) requires a canonical identity.

**Actors**: `cpt-insightspec-actor-tt-silver-identity-manager`

#### Supporting Tables

- [ ] `p2` - **ID**: `cpt-insightspec-fr-tt-silver-supporting-tables`

The system **MUST** provide unified Silver tables for worklogs, comments, sprints, projects, users, issue links, and collection runs alongside the field history table.

**Rationale**: Field history alone is insufficient for complete analytics. Worklogs provide actual time invested, comments provide collaboration signals, sprints provide velocity context.

**Actors**: `cpt-insightspec-actor-tt-silver-data-analyst`

## 6. Non-Functional Requirements

### 6.1 Module-Specific NFRs

#### Incremental Processing

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-tt-silver-incremental`

The system **MUST** support incremental processing — only new events since the last sync run are processed.

**Threshold**: Processing time for an incremental run MUST be proportional to the number of new events, not the total history size.

**Rationale**: Full reprocessing of all historical events on every sync is not scalable for large instances with millions of changelog entries.

#### Deduplication

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-tt-silver-dedup`

The system **MUST** guarantee idempotent re-ingestion — reprocessing the same events MUST NOT create duplicate records.

**Threshold**: Zero duplicate records after any number of re-runs with the same input data.

**Rationale**: Overlapping sync windows and retry logic can re-deliver the same events.

#### Data Freshness

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-tt-silver-freshness`

Silver data **SHOULD** reflect source changes within one sync cycle (default: daily).

**Threshold**: Silver tables updated within 1 hour of Airbyte sync completion.

**Rationale**: Daily reporting cadence is sufficient for team productivity metrics.

### 6.2 NFR Exclusions

| Category | Reason |
|----------|--------|
| Real-time processing | Daily sync cadence is sufficient for productivity analytics |
| High availability | Internal analytics system; scheduled downtime acceptable |
| GDPR/compliance | Handled at the platform level, not per-connector Silver layer |
| UI performance | No UI in this layer; consumed by dbt and downstream dashboards |

## 7. Public Library Interfaces

### 7.1 Public API Surface

Not applicable — the Silver layer exposes ClickHouse tables, not API endpoints. Consumers query tables directly via SQL.

### 7.2 External Integration Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-contract-tt-silver-clickhouse`

**Direction**: Outbound (Silver tables -> consumers)

**Protocol/Format**: ClickHouse SQL over native protocol or HTTP

**Compatibility**: Table schemas are versioned via `_version` column. Schema changes require migration scripts.

- [ ] `p1` - **ID**: `cpt-insightspec-contract-tt-silver-identity`

**Direction**: Outbound (Silver -> Identity Manager)

**Protocol/Format**: `task_tracker_users.email` -> Identity Manager -> `person_id`

**Compatibility**: Identity Manager contract defined in Identity Resolution specs.

## 8. Use Cases

#### UC-001: Query Cycle Time for an Issue

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-tt-silver-cycle-time`

**Actor**: `cpt-insightspec-actor-tt-silver-data-analyst`

**Preconditions**: Issue has status field history with full values in `task_tracker_field_history`.

**Main Flow**:
1. Analyst queries field history for `field_name = 'status'` (or source-specific status field)
2. Finds first event where `value_displays = ['In Progress']`
3. Finds first event where `value_displays = ['Done']`
4. Calculates difference as cycle time

**Postconditions**: Cycle time computed from two simple row lookups.

**Alternative Flows**:
- Issue never reached "Done": cycle time is NULL
- Issue reopened: multiple In Progress -> Done transitions; use first or last per business rules

#### UC-002: Sprint Carry-Over Analysis

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-tt-silver-sprint-carryover`

**Actor**: `cpt-insightspec-actor-tt-silver-data-analyst`

**Preconditions**: Sprint field is multi-value; full value arrays show all sprints the issue belongs to at each change.

**Main Flow**:
1. Analyst queries field history for `field_name = 'Sprint'` (or source-specific)
2. Finds events where `length(value_ids) > 1` — issue belongs to multiple sprints simultaneously
3. Identifies carry-over: issue was not completed in Sprint N and was added to Sprint N+1

**Postconditions**: Carry-over issues identified per sprint.

#### UC-003: Cross-System Workload Report

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-tt-silver-cross-system`

**Actor**: `cpt-insightspec-actor-tt-silver-eng-manager`

**Preconditions**: Field history populated from multiple sources; identity resolution completed.

**Main Flow**:
1. Manager queries field history joined with identity-resolved `person_id`
2. Filters by `field_name` = assignee across all `data_source` values
3. Aggregates by `person_id` to see total assigned issues per person across YouTrack and Jira

**Postconditions**: Unified workload view across task tracking systems.

## 9. Acceptance Criteria

- [ ] Field history records contain full values (not deltas) for all tracked fields
- [ ] Initial state at issue creation is captured as field history records
- [ ] Single-value and multi-value fields are correctly classified
- [ ] Four source systems (YouTrack, Jira, GitHub Projects V2, Azure DevOps) produce data in the unified schema
- [ ] Identity resolution maps `author_id` to `person_id` for all sources
- [ ] Incremental processing handles only new events per sync run
- [ ] Re-ingestion of same events produces no duplicates

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Bronze connectors | YouTrack, Jira, GitHub, Azure DevOps connectors producing Bronze data | Required — no Silver without Bronze |
| Field metadata APIs | Source system APIs for field type information | Required — field cardinality cannot be inferred from changelog |
| Identity Manager | Resolves email -> person_id | Required for Silver Step 2 |
| ClickHouse | Storage engine for all Silver tables | Required |
| dbt-clickhouse | SQL transformation framework | Required |
| Argo Workflows | Pipeline orchestration | Required |

## 11. Assumptions

- Source task tracker APIs remain stable and accessible
- Field metadata APIs accurately reflect current field types
- YouTrack field type changes (enum[1] -> enum[*]) are rare in production
- GitHub Projects V2 will eventually provide a changelog API (current polling approach is interim)
- Daily sync cadence is sufficient for team productivity analytics
- Issue volume per source instance is under 1M issues with under 10M changelog entries

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| GitHub Projects V2 has no changelog API | Synthetic deltas from snapshot diffing may miss rapid changes between syncs | Increase sync frequency for GitHub sources; document data completeness limitations |
| YouTrack field type changes mid-stream | Incorrect full values if type change is not detected promptly | Query field metadata every sync run; maintain `field_type_history` |
| Jira email suppression | Users without email cannot be identity-resolved | Fall back to Atlassian `account_id` for within-ecosystem joins (OQ-TT-2) |
| Large changelog volumes (>10M events) | Enrich processing time may exceed acceptable limits | Incremental processing; only process new events since last cursor |
| Story points field detection across instances | Different field IDs per project style and instance | Defer to Silver/dbt extraction based on project metadata (OQ-TT-1) |
