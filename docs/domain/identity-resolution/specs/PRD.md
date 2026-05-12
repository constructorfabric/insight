# PRD — Identity Resolution

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
  - [5.1 Phase 1 — MVP: dbt Seed (p1)](#51-phase-1--mvp-dbt-seed-p1)
  - [5.2 Phase 2 — Bootstrap Pipeline (p1)](#52-phase-2--bootstrap-pipeline-p1)
  - [5.3 Phase 3 — Matching & Workflows (p2)](#53-phase-3--matching--workflows-p2)
  - [5.4 Late Phase — Merge/Split & GDPR (p3)](#54-late-phase--mergesplit--gdpr-p3)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Bootstrap New Connector Data](#bootstrap-new-connector-data)
  - [Resolve Alias (Hot Path)](#resolve-alias-hot-path)
  - [Review Unmapped Aliases](#review-unmapped-aliases)
  - [Merge Two Person Alias Sets](#merge-two-person-alias-sets)
  - [GDPR Alias Purge](#gdpr-alias-purge)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

Identity Resolution maps disparate identity signals — emails, usernames, employee IDs, platform-specific handles — from all connected source systems to canonical person records. It enables cross-system analytics by answering one question: "Which person does this account belong to?" Without it, a Git committer `aivanova`, a Jira assignee `anna.ivanova@acme.com`, and a BambooHR employee `E123` remain three unrelated identities, preventing any meaningful cross-platform productivity or collaboration analysis.

### 1.2 Background / Problem Statement

Insight connects to 10+ external platforms (GitLab, GitHub, Jira, YouTrack, BambooHR, Zoom, M365, Zulip, etc.). Each platform uses its own account model — some identify users by email, others by username, numeric ID, or display name. A single human may appear as 5-15 different identities across these systems.

**Current state**: The original identity resolution monolith handled person records, alias mapping, org hierarchy, golden record assembly, and GDPR deletion in a single design. As part of the domain-split initiative, identity resolution is now scoped to alias-to-person mapping only. Person attributes, org hierarchy, and availability belong to their respective domains.

**Key problems solved**:
- **Fragmented identity**: Commits, issues, messages, and HR records cannot be attributed to the same person without alias resolution
- **Connector diversity**: Each new connector introduces new alias types and naming conventions — the system must handle this uniformly
- **Confidence and safety**: Auto-linking wrong aliases creates corrupt analytics; the system must be conservative and auditable
- **Operational cost**: Without automated bootstrap, operators must manually map every identity — unsustainable at scale

**Target users**: Platform operators managing identity mappings; analytics consumers relying on accurate `person_id` attribution; connectors writing alias observations.

### 1.3 Goals (Business Outcomes)

| Goal | Success Criteria |
|---|---|
| Automated alias resolution | **Baseline**: 0% auto-resolved. **Target**: >= 80% of aliases auto-resolved within 30 min of connector sync. **Timeframe**: Within 2 sprints of Phase 2 deployment. |
| Zero false-positive auto-links | **Baseline**: N/A (new system). **Target**: 0 false-positive auto-links in production over 90-day window. **Timeframe**: Ongoing from Phase 2 launch. |
| Operator efficiency | **Baseline**: 100% manual mapping. **Target**: Operator reviews < 20% of aliases (unmapped queue only). **Timeframe**: Within 30 days of Phase 3 deployment. |
| Cross-platform analytics enablement | **Baseline**: Per-platform siloed dashboards. **Target**: 100% of Gold analytics queries use resolved `person_id`. **Timeframe**: Within 1 sprint of Phase 1 deployment. |
| Audit trail completeness | **Baseline**: No merge/split tracking. **Target**: 100% of merge/split operations have reversible audit records. **Timeframe**: From late-phase deployment. |

### 1.4 Glossary

| Term | Definition |
|---|---|
| Alias | An `(value_type, value)` pair identifying a person in a specific source system (e.g., `email:anna@acme.com`) |
| Alias type | Category of identity signal: canonical `id`, `email`, `username`, `display_name`; known custom `employee_id`, `platform_id` (deprecated for Cursor/Claude Admin in favor of `id`, see ADR-0002) |
| Bootstrap input | A row in `identity_inputs` representing one changed alias observation from one connector |
| Confidence score | Numeric value (0.0–1.0) representing the MatchingEngine's certainty that an alias belongs to a person |
| Auto-link | Automatic creation of an alias mapping when confidence >= 1.0 |
| Unmapped alias | An alias that could not be resolved above the confidence threshold; queued for operator review |
| Alias conflict | When the same `(value_type, value)` is claimed by two different persons |
| Merge | Combining two person alias sets under a single `person_id` |
| Split | Reversing a merge by restoring alias mappings from an audit snapshot |
| Hot path | Direct alias lookup in `aliases` table (~90% of resolutions) |
| Cold path | MatchingEngine rule evaluation when hot path misses |
| Person domain | Separate domain owning the **golden record projection** of persons (and person-level attributes such as availability, conflicts). The MariaDB `persons` identity-attribute history table itself is owned by the identity-resolution domain (see DESIGN §3.7 and ADR-0006); the Person domain reads from it |
| Org-chart domain | Separate domain owning `org_units` and `person_assignments` |

---

## 2. Actors

> **Note**: Stakeholder needs are managed at project/task level by steering committee. Document **actors** (users, systems) that interact with this module.

### 2.1 Human Actors

#### Operator

**ID**: `cpt-ir-actor-operator`

**Role**: Reviews unmapped aliases, resolves alias conflicts, manages match rule configuration, performs merge/split operations, and handles GDPR purge requests. Typically a platform administrator with knowledge of the organization's systems and personnel.

**Needs**: A clear queue of unresolved aliases with suggested matches; ability to link, ignore, or create new persons; visibility into merge/split history for audit; configurable match rules.

### 2.2 System Actors

#### Connector

**ID**: `cpt-ir-actor-connector`

**Role**: External system connector (e.g., BambooHR, GitLab, Jira) that syncs data from source platforms. Writes alias observations to `identity_inputs` during its sync pipeline, providing raw identity signals for resolution.

#### BootstrapJob

**ID**: `cpt-ir-actor-bootstrap-job`

**Role**: Scheduled job (Argo Workflow) that reads unprocessed rows from `identity_inputs`, normalizes alias values, evaluates matching rules, and creates or updates entries in the `aliases` table. Runs after each connector sync cycle.

#### MatchingEngine

**ID**: `cpt-ir-actor-matching-engine`

**Role**: Rule evaluation engine invoked by BootstrapJob and ResolutionService on the cold path. Loads enabled `match_rules`, computes composite confidence scores, and returns candidate `person_id` with confidence level. Does not write to tables directly.

#### Analytics Pipeline

**ID**: `cpt-ir-actor-analytics-pipeline`

**Role**: Downstream consumer (dbt models, dashboards) that resolves `person_id` by joining Silver tables against the `aliases` table or ClickHouse Dictionary. Depends on alias data being accurate and up-to-date for cross-platform attribution.

---

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **Storage**: Analytical identity-resolution tables reside in ClickHouse (`identity_inputs`, `aliases`, `match_rules`, `unmapped`, `conflicts`, `merge_audits`, `alias_gdpr_deleted`). The identity-attribute observation history `persons` and its derived SCD2 cache `account_person_map` reside in MariaDB and are owned by this domain (see DESIGN §3.7 and [ADR-0006](../../ingestion/specs/ADR/0006-service-owned-migrations.md)). The Person and Org-Chart domains remain ClickHouse-only.
- **Orchestration**: BootstrapJob runs as an Argo WorkflowTemplate on a Kind K8s cluster (per PR #45).
- **Naming**: All tables and columns follow PR #55 glossary conventions (see Glossary and DESIGN §2.2).
- **Temporal model**: Half-open intervals `[effective_from, effective_to)`. `BETWEEN` prohibited on temporal columns. Zero sentinel (`'1970-01-01'`) replaces NULL for ClickHouse compatibility.

---

## 4. Scope

### 4.1 In Scope

- Bootstrap mechanism: `identity_inputs` ingestion, BootstrapJob processing pipeline
- Alias store: `aliases` table, alias CRUD, temporal ownership tracking
- Resolution API: hot-path and cold-path alias-to-person resolution
- Matching engine: configurable `match_rules`, confidence scoring, normalization pipeline
- Unmapped alias queue: operator review workflow (list, resolve, ignore)
- Alias conflict detection: when same alias value maps to multiple persons
- Merge/split operations with full audit trail via `merge_audits` (late phase)
- GDPR alias deletion: move to `alias_gdpr_deleted`, remove from `aliases` (late phase)
- ClickHouse Dictionary for hot-path analytical lookups (optional optimization)
- **Persons identity-attribute history**: MariaDB `persons` table and its one-shot seed from `identity_inputs` (see DESIGN §3.7 and ADR-0002). Schema / CRUD / seed are owned by this domain; the Person domain reads the resulting rows to build its golden record.

### 4.2 Out of Scope

- **Person golden record**: assembly of the best-value `persons` record per attribute with source-priority rules, person-level conflict detection — see person domain PRD. (This domain records raw observations; the person domain projects them into a single golden record.)
- **Org hierarchy**: `org_units`, `person_assignments`, SCD Type 2 history — see org-chart domain PRD
- **Connector implementation**: How connectors sync data from external platforms — see connector specifications
- **Permission / RBAC**: Access control, data visibility rules — see permissions domain
- **Metric aggregation**: Gold-layer dashboards, activity summaries — see analytics domain
- **SCD Type 2 for persons/org_units**: Implemented via dbt macros in respective domains

---

## 5. Functional Requirements

> **Testing strategy**: All requirements verified via automated tests (unit, integration, e2e) targeting 90%+ code coverage unless otherwise specified. Document verification method only for non-test approaches (analysis, inspection, demonstration).

### 5.1 Phase 1 — MVP: dbt Seed (p1)

#### Seed Aliases from HR Bronze Data

- [ ] `p1` - **ID**: `cpt-ir-fr-seed-aliases`

The system **MUST** create initial alias records in the `aliases` table from HR Bronze data via dbt seed models, mapping `employee_id` and `email` alias types to person records created by the person domain.

**Rationale**: Without initial aliases, no resolution can happen. HR data (BambooHR, Workday) provides the most reliable identity anchors — employee IDs and work emails.

**Actors**: `cpt-ir-actor-analytics-pipeline`

#### Resolve Alias to Person

- [ ] `p1` - **ID**: `cpt-ir-fr-resolve-alias`

The system **MUST** resolve an `(insight_tenant_id, insight_source_type, insight_source_id, value_type, value)` tuple to a `person_id` by looking up active, non-deleted rows in the `aliases` table. The source identifier is required because identifiers like `value_type='id'` (and platform usernames) are unique only within a single source — the same `source_account_id` value can refer to different persons across BambooHR vs Cursor, and the same email can appear with different `source_account_id`s in different platforms. If found, the system **MUST** return the `person_id` and confidence. If not found, it **MUST** return a null `person_id` with status `unmapped`.

**Rationale**: This is the core capability — every downstream analytics query depends on resolving aliases to persons.

**Actors**: `cpt-ir-actor-analytics-pipeline`

#### Batch Alias Resolution

- [ ] `p1` - **ID**: `cpt-ir-fr-batch-resolve`

The system **MUST** support batch resolution of multiple aliases in a single request, returning a `person_id` (or null) for each input alias.

**Rationale**: Silver step 2 enrichment processes millions of rows; one-by-one resolution is prohibitively slow.

**Actors**: `cpt-ir-actor-analytics-pipeline`

#### Tenant Isolation

- [ ] `p1` - **ID**: `cpt-ir-fr-tenant-isolation`

The system **MUST** isolate alias data by `insight_tenant_id`. A resolution request for tenant A **MUST NOT** return aliases belonging to tenant B.

**Rationale**: Multi-tenant deployments require strict data isolation to prevent cross-tenant data leaks.

**Actors**: `cpt-ir-actor-analytics-pipeline`, `cpt-ir-actor-operator`

#### Persons — Field-Level Identity Attribute History (MariaDB)

- [ ] `p1` - **ID**: `cpt-ir-fr-persons-history`

The system **MUST** maintain a `persons` table in MariaDB that stores the history of identity field changes per person in an append-only SCD-style log. Each row represents one observed field value (`value_type`, `value`) from one source (`insight_source_type`, `insight_source_id`) assigned to a person (`person_id`) at a specific time (`created_at`). Updating a field **MUST** insert a new row rather than mutate an existing one; the current value is the row with the latest `created_at` for the `(insight_tenant_id, person_id, value_type)` triple.

Schema (the column split by `value_type` into `value_id` / `value_full_text` / `value`, the generated `value_effective` for display and `value_hash` for the natural-key UNIQUE, the SCD2 shape of `account_person_map`, and `TIMESTAMP(6)` precision) is specified in the identity-resolution DESIGN §3.7 and in [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md). The PRD intentionally states only the behavioural contract.

**Rationale**: Downstream backend services (Analytics API, Identity Resolution service) need a CRUD-accessible, temporal view of person attributes from heterogeneous sources — ClickHouse is optimised for analytical reads, not operator-driven edits. MariaDB with row-level history supports conflict-resolution UX, audit trails, and operator-driven corrections.

**Actors**: `cpt-ir-actor-analytics-pipeline`, `cpt-ir-actor-operator`

#### Initial Seed of Persons from identity_inputs

- [ ] `p1` - **ID**: `cpt-ir-fr-persons-initial-seed`

The system **MUST** provide a re-runnable bootstrap mechanism that populates `persons` from `identity_inputs` with the following behavioural guarantees:

- **Email as identity anchor**: each source-account's email observation uniquely identifies a person within a tenant; source-accounts without an email **MUST** be skipped.
- **Stable identity assignment**: within an initial-bootstrap run, source-accounts sharing a normalised email within a tenant are assigned one randomly-minted `person_id` (UUIDv7); once assigned, the binding is never re-derived from mutable attributes and survives re-runs. See [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md).
- **Idempotent re-run**: running the bootstrap on identical input **MUST NOT** add any rows; running after a new connector sync **MUST** add only the newly observed rows without losing or duplicating prior ones.
- **Non-destructive**: the bootstrap **MUST NOT** delete, truncate, or otherwise remove rows from `persons`. Re-seeding from scratch is an explicit operator action outside the bootstrap.

**Rationale**: Bootstraps the `persons` table with every person who has an email anchor across configured connectors. Email is the single authoritative cross-source identity key in Phase 1 (matching rules come in Phase 3). The deterministic-identity + idempotent-re-run guarantees together mean the bootstrap is safe to execute after every connector sync without duplicating data or wiping operator-authored rows.

Schema of `persons`, UNIQUE-key structure, `person_id` minting (random UUIDv7), and script layout are specified in the identity-resolution DESIGN §3.7 "Table: `persons`" and in [ADR-0002](ADR/0002-stable-person-id-via-persons-observations.md).

**Actors**: `cpt-ir-actor-analytics-pipeline`

### 5.2 Phase 2 — Bootstrap Pipeline (p1)

#### Accept Alias Observations from Connectors

- [x] `p1` - **ID**: `cpt-ir-fr-accept-bootstrap-inputs`

The system **MUST** accept alias observation records into the `identity_inputs` table. Each record **MUST** include: `insight_tenant_id`, `insight_source_id`, `insight_source_type`, `source_account_id`, `value_type`, `value`, `value_field_name`, `operation_type` (UPSERT or DELETE).

**Rationale**: Connectors need a uniform write target for identity signals. The `identity_inputs` table decouples connector sync from alias resolution timing.

**Actors**: `cpt-ir-actor-connector`

#### Process Bootstrap Inputs Incrementally

- [x] `p1` - **ID**: `cpt-ir-fr-bootstrap-incremental`

The BootstrapJob **MUST** process `identity_inputs` rows incrementally, reading only rows with `_synced_at` greater than the last processing watermark. It **MUST** update the watermark after each successful run.

**Rationale**: Connectors sync continuously; the bootstrap pipeline must process only new observations to avoid re-processing the entire history on each run.

**Actors**: `cpt-ir-actor-bootstrap-job`

#### Normalize Alias Values

- [ ] `p1` - **ID**: `cpt-ir-fr-normalize-aliases`

The BootstrapJob **MUST** normalize alias values before matching: `email` and `username` types **MUST** be lowercased and trimmed; all other types **MUST** be trimmed. Raw values in `identity_inputs` **MUST** be preserved unchanged.

**Rationale**: Case differences and whitespace in emails/usernames cause false negatives in matching. Normalization ensures consistent lookups.

**Actors**: `cpt-ir-actor-bootstrap-job`

#### Create Alias on Exact Match

- [ ] `p1` - **ID**: `cpt-ir-fr-create-alias-exact`

When the BootstrapJob finds no existing alias for a normalized `(value_type, value, insight_tenant_id)` and the MatchingEngine returns confidence >= 1.0, the system **MUST** auto-create an alias record in the `aliases` table linking to the matched `person_id`.

**Rationale**: High-confidence matches (exact email, exact employee ID) should be linked automatically without operator intervention to achieve the >= 80% auto-resolution goal.

**Actors**: `cpt-ir-actor-bootstrap-job`, `cpt-ir-actor-matching-engine`

#### Route Low-Confidence Aliases to Unmapped Queue

- [ ] `p1` - **ID**: `cpt-ir-fr-route-unmapped`

When the MatchingEngine returns confidence < 1.0 for an alias, the system **MUST** insert the alias into the `unmapped` table. If confidence is 0.50–0.99, the system **MUST** include the `suggested_person_id` and `suggestion_confidence`. If confidence < 0.50, the system **MUST** insert with status `pending` and no suggestion.

**Rationale**: Aliases below the auto-link threshold must not be silently dropped; they need operator review to prevent identity gaps.

**Actors**: `cpt-ir-actor-bootstrap-job`, `cpt-ir-actor-matching-engine`

#### Track Alias Observations Over Time

- [ ] `p1` - **ID**: `cpt-ir-fr-track-observations`

When an alias already exists in the `aliases` table for the same `(value_type, value, insight_source_id, insight_tenant_id)`, the BootstrapJob **MUST** update `last_observed_at` to the current timestamp. It **SHOULD** update `source_account_id` if changed.

**Rationale**: Tracking when aliases were last confirmed helps identify stale mappings and provides audit context.

**Actors**: `cpt-ir-actor-bootstrap-job`

#### Idempotent Bootstrap Runs

- [ ] `p1` - **ID**: `cpt-ir-fr-bootstrap-idempotent`

Re-running the BootstrapJob on the same `identity_inputs` data **MUST NOT** create duplicate alias records. The system **MUST** deduplicate on the natural key `(insight_tenant_id, value_type, value, insight_source_id)`.

**Rationale**: Connector retries and Argo Workflow restarts must be safe. Duplicate aliases would corrupt resolution results and inflate metrics.

**Actors**: `cpt-ir-actor-bootstrap-job`

### 5.3 Phase 3 — Matching & Workflows (p2)

#### Configurable Match Rules

- [ ] `p2` - **ID**: `cpt-ir-fr-configurable-rules`

The system **MUST** allow operators to view, enable/disable, and adjust weights of match rules via the API. Each rule **MUST** have a `rule_type`, `phase`, `condition_type`, `weight`, `is_enabled`, and `sort_order`.

**Rationale**: Different deployments have different identity landscapes. Operators need to tune matching rules for their specific source mix without code changes.

**Actors**: `cpt-ir-actor-operator`

#### Three-Phase Matching Pipeline

- [ ] `p2` - **ID**: `cpt-ir-fr-three-phase-matching`

The MatchingEngine **MUST** evaluate rules in three ordered phases: B1 (deterministic — exact email, exact HR ID), B2 (normalization and cross-system — case-insensitive email, domain aliases, cross-system username), B3 (fuzzy — Jaro-Winkler, Soundex). The system **MUST** compute a composite confidence score from weighted rule matches.

**Rationale**: Phased matching provides escalating specificity. Deterministic rules run first for speed; fuzzy rules run last and only when deterministic matching fails.

**Actors**: `cpt-ir-actor-matching-engine`

#### No Fuzzy Auto-Link

- [ ] `p2` - **ID**: `cpt-ir-fr-no-fuzzy-autolink`

Fuzzy matching rules (Phase B3) **MUST NEVER** trigger automatic alias creation regardless of confidence score. They **MUST** only generate suggestions routed to the unmapped queue for operator review.

**Rationale**: Production experience showed fuzzy name matching produced false-positive merges. This constraint is non-negotiable.

**Actors**: `cpt-ir-actor-matching-engine`

#### Operator Unmapped Queue Management

- [ ] `p2` - **ID**: `cpt-ir-fr-unmapped-management`

The system **MUST** allow operators to: (a) list unmapped aliases filtered by status, (b) link an unmapped alias to an existing person, (c) create a new person from an unmapped alias (via person domain), (d) mark an unmapped alias as ignored. Each resolution action **MUST** update the `unmapped` record with `resolved_person_id`, `resolved_at`, `resolved_by_person_id`, and `resolution_type`.

**Rationale**: Not all aliases can be auto-resolved. Operators need an efficient workflow to clear the unmapped queue and maintain data quality.

**Actors**: `cpt-ir-actor-operator`

#### Alias Conflict Detection

- [ ] `p2` - **ID**: `cpt-ir-fr-alias-conflict-detection`

When the BootstrapJob encounters a new alias observation that matches an alias already owned by a different person, the system **MUST** create a conflict record in the `conflicts` table with both `person_id_a`, `person_id_b`, the conflicting `value_type`/`value`, and source IDs.

**Rationale**: The same email or username claimed by two different persons indicates a data quality issue that requires operator attention.

**Actors**: `cpt-ir-actor-bootstrap-job`

#### Manual Alias Management

- [ ] `p2` - **ID**: `cpt-ir-fr-manual-alias-crud`

The system **MUST** allow operators to: (a) add an alias to a person manually, (b) deactivate an alias (set `is_active = 0`), (c) list all aliases for a given person. Each action **MUST** be logged in `merge_audits` with `action = 'alias_add'` or `'alias_remove'`.

**Rationale**: Automated resolution covers most cases, but operators need escape hatches for edge cases — manually added email addresses, correcting mislinked aliases.

**Actors**: `cpt-ir-actor-operator`

#### Auto-Resolve Unmapped on New Alias

- [ ] `p2` - **ID**: `cpt-ir-fr-auto-resolve-unmapped`

When the BootstrapJob creates a new alias, the system **SHOULD** check the `unmapped` table for pending entries matching the same `(value_type, value, insight_tenant_id)` and auto-resolve them to the newly linked person.

**Rationale**: As more connectors sync, previously unmapped aliases may become resolvable. Auto-resolution reduces operator queue size.

**Actors**: `cpt-ir-actor-bootstrap-job`

### 5.4 Late Phase — Merge/Split & GDPR (p3)

#### Merge Person Alias Sets

- [ ] `p3` - **ID**: `cpt-ir-fr-merge`

The system **MUST** allow an operator to merge all aliases from `source_person_id` to `target_person_id`. The merge operation **MUST** snapshot the alias state before and after in `merge_audits`, reassign all source aliases, and run alias conflict detection on the target.

**Rationale**: Operators occasionally discover that two person records represent the same individual. Merge combines their alias sets with full audit trail.

**Actors**: `cpt-ir-actor-operator`

#### Split (Rollback Merge)

- [ ] `p3` - **ID**: `cpt-ir-fr-split`

The system **MUST** allow an operator to reverse a previous merge by restoring alias mappings from `merge_audits.snapshot_before`. The system **MUST NOT** allow a split on an already-rolled-back audit record.

**Rationale**: Merges can be wrong. Reversibility is essential for data integrity and operator confidence.

**Actors**: `cpt-ir-actor-operator`

#### Merge/Split Audit Trail

- [ ] `p3` - **ID**: `cpt-ir-fr-merge-audit`

Every merge, split, alias_add, and alias_remove operation **MUST** be recorded in `merge_audits` with: `action`, `target_person_id`, `source_person_id`, `snapshot_before` (JSON), `snapshot_after` (JSON), `reason`, `actor_person_id`, `performed_at`.

**Rationale**: Auditability is required for compliance, debugging, and building operator trust. Full snapshots enable rollback.

**Actors**: `cpt-ir-actor-operator`

#### GDPR Alias Purge

- [ ] `p3` - **ID**: `cpt-ir-fr-gdpr-purge`

The system **MUST** support GDPR hard erasure for a person's aliases: move all alias records to `alias_gdpr_deleted`, remove from `aliases` (set `is_deleted = 1`), and ensure the alias is no longer resolvable via any path (hot or cold).

**Rationale**: Legal compliance with right-to-erasure requests. Alias data contains PII (emails, names, employee IDs).

**Actors**: `cpt-ir-actor-operator`

#### Idempotent Merge/Split Operations

- [ ] `p3` - **ID**: `cpt-ir-fr-idempotent-mutations`

All mutating API endpoints (merge, split, purge, alias add/remove) **MUST** support idempotency keys (`Idempotency-Key` header, 24h TTL). Replaying a request with the same key **MUST** return the original result without re-executing the operation.

**Rationale**: Network retries and operator double-clicks must be safe. ClickHouse lacks ACID transactions, so idempotency is the primary safety mechanism.

**Actors**: `cpt-ir-actor-operator`

---

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Alias Lookup Latency

- [ ] `p1` - **ID**: `cpt-ir-nfr-alias-lookup-latency`

The system **MUST** resolve a single alias to `person_id` via the hot path in < 50 ms at p99 under normal load.

**Threshold**: p99 latency < 50 ms for `POST /resolve` when alias exists in `aliases` table, measured at 1000 req/s sustained.

**Rationale**: Resolution is on the critical path for Silver step 2 enrichment. High latency blocks analytical pipeline throughput.

#### Bootstrap Throughput

- [ ] `p1` - **ID**: `cpt-ir-nfr-bootstrap-throughput`

The BootstrapJob **MUST** process at least 100,000 `identity_inputs` rows per run within 30 minutes.

**Threshold**: >= 100K rows processed in <= 30 min on standard cluster resources (0.5 CPU, 512 MB RAM).

**Rationale**: Large connector syncs (BambooHR with 50K employees, GitLab with 100K users) must complete within the dashboard visibility SLA.

#### Bootstrap Idempotency

- [ ] `p1` - **ID**: `cpt-ir-nfr-bootstrap-idempotency`

Re-running the BootstrapJob on identical input **MUST** produce identical output — zero net new alias rows, zero net deleted rows.

**Threshold**: After 3 consecutive runs on unchanged data, `SELECT count() FROM aliases` returns the same value.

**Rationale**: System restarts, Argo retries, and operational re-runs must be safe.

#### No Fuzzy Auto-Link Safety

- [ ] `p2` - **ID**: `cpt-ir-nfr-no-fuzzy-autolink`

The system **MUST** produce zero false-positive auto-links from fuzzy matching rules over any 90-day production window.

**Threshold**: 0 auto-created aliases traced to Phase B3 fuzzy rules.

**Rationale**: False merges corrupt analytics and are extremely costly to unwind. This is a hard safety constraint.

#### Tenant Data Isolation

- [ ] `p1` - **ID**: `cpt-ir-nfr-tenant-isolation`

A resolution request for tenant A **MUST NOT** return data from tenant B under any circumstances, including cache hits, Dictionary lookups, and error responses.

**Threshold**: 0 cross-tenant data leaks in penetration testing.

**Rationale**: Multi-tenant SaaS compliance requirement.

#### GDPR Erasure Completeness

- [ ] `p3` - **ID**: `cpt-ir-nfr-gdpr-erasure`

After a GDPR purge, the purged aliases **MUST NOT** be resolvable via any path (API, Dictionary, direct table query on `aliases`) within 60 minutes.

**Threshold**: Purged alias returns null from `POST /resolve` within 60 min of purge. Dictionary reload completes within TTL.

**Rationale**: Legal compliance with right-to-erasure. Delayed purge visibility is a regulatory risk.

#### Merge/Split Reversibility

- [ ] `p3` - **ID**: `cpt-ir-nfr-merge-reversibility`

Every merge operation **MUST** be fully reversible via split. After a merge-then-split round-trip, the alias state **MUST** be identical to the pre-merge state.

**Threshold**: 100% round-trip fidelity verified by comparing `snapshot_before` with post-split alias state.

**Rationale**: Operator confidence and data integrity. Irreversible merges would make operators reluctant to act.

### 6.2 NFR Exclusions

- **High availability / clustering**: Identity resolution is not on the real-time serving path for end users. ClickHouse cluster availability is managed at infrastructure level, not by this domain.
- **Sub-second consistency**: The analytical pipeline tolerates 30-60s staleness (Dictionary TTL). Real-time consistency is not required.
- **Encryption at rest**: Handled by ClickHouse infrastructure configuration, not by application-level encryption in this domain.

---

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Identity Resolution REST API

- [ ] `p1` - **ID**: `cpt-ir-interface-resolution-api`

**Type**: REST API (HTTP/JSON)

**Stability**: stable

**Description**: Primary interface for alias resolution, merge/split operations, unmapped queue management, match rule configuration, and GDPR purge. Base path: `/api/identity/`.

**Breaking Change Policy**: Endpoint paths and response shapes are versioned; breaking changes require major version bump (`/api/v2/identity/`).

#### ClickHouse Dictionary (Analytical Lookup)

- [ ] `p2` - **ID**: `cpt-ir-interface-ch-dictionary`

**Type**: ClickHouse Dictionary

**Stability**: stable

**Description**: Optional read-only interface for analytical queries. Keyed by `(insight_tenant_id, value_type, value)`, returns `person_id`. Reload TTL 30-60s. Used by dbt models and dashboards for Silver step 2 enrichment.

**Breaking Change Policy**: Dictionary key structure changes require downstream dbt model updates.

### 7.2 External Integration Contracts

#### Bootstrap Inputs Write Contract

- [x] `p1` - **ID**: `cpt-ir-contract-bootstrap-inputs`

**Direction**: required from client (connectors)

**Protocol/Format**: ClickHouse INSERT (native protocol or HTTP interface)

**Description**: Connectors **MUST** write alias observations to the `identity_inputs` table with all required fields (`insight_tenant_id`, `insight_source_id`, `insight_source_type`, `source_account_id`, `value_type`, `value`, `value_field_name`, `operation_type`). The `value_field_name` **MUST** be fully-qualified: `bronze_{descriptor.name}.{table}.{field}[.json_path]`.

**Compatibility**: Additive columns are backward-compatible. Removing or renaming required columns is a breaking change.

#### Person Domain Cross-Reference Contract

- [ ] `p1` - **ID**: `cpt-ir-contract-person-domain`

**Direction**: provided by library (identity resolution provides `aliases.person_id`)

**Protocol/Format**: Logical FK — `aliases.person_id` references `persons.person_id` (the stable UUIDv7, not the auto-increment observation row PK)

**Description**: The `aliases` table provides the authoritative mapping from identity signals to person records. The person domain owns person creation; identity resolution links aliases to existing persons. The `person_id` column in `aliases` is the primary integration point.

**Compatibility**: The `person_id` UUID format is stable. Column name changes are breaking.

---

## 8. Use Cases

### Bootstrap New Connector Data

- [ ] `p1` - **ID**: `cpt-ir-usecase-bootstrap`

**Actor**: `cpt-ir-actor-bootstrap-job`, `cpt-ir-actor-connector`

**Preconditions**:
- Connector has completed a sync and written rows to `identity_inputs`
- BootstrapJob is triggered (Argo Workflow post-sync)
- Person records exist in person domain (seeded by dbt or previous bootstrap)

**Main Flow**:
1. BootstrapJob reads `identity_inputs` rows where `_synced_at > last_watermark`
2. For each row, normalize `value` (lowercase/trim for email/username)
3. Look up existing alias in `aliases` for `(tenant, value_type, normalized_value)`
4. If alias exists for same person: update `last_observed_at`
5. If alias does not exist: invoke MatchingEngine with the alias
6. If confidence >= 1.0: create alias in `aliases` table, auto-resolve matching unmapped entries
7. If confidence < 1.0: insert into `unmapped` (with suggestion if confidence >= 0.50)
8. Update processing watermark

**Postconditions**:
- New aliases created in `aliases` table for high-confidence matches
- Low-confidence aliases queued in `unmapped` for operator review
- Processing watermark advanced

**Alternative Flows**:
- **Alias exists for different person**: ConflictDetector creates a `conflicts` record; alias is NOT auto-created
- **BootstrapJob fails mid-run**: Watermark not updated; safe to retry (idempotent)
- **No matching person in person domain**: Alias routed to `unmapped` as pending

---

### Resolve Alias (Hot Path)

- [ ] `p1` - **ID**: `cpt-ir-usecase-resolve-hot`

**Actor**: `cpt-ir-actor-analytics-pipeline`

**Preconditions**:
- Alias exists in `aliases` table with `is_active = 1` and `is_deleted = 0`

**Main Flow**:
1. Caller sends `POST /resolve` with `value_type`, `value`, `insight_source_id`, `insight_tenant_id`
2. System queries `aliases` table for active, non-deleted match
3. System returns `{person_id, confidence: 1.0, status: "resolved"}`

**Postconditions**:
- Caller has `person_id` for downstream processing

**Alternative Flows**:
- **Alias not found (cold path)**: System invokes MatchingEngine; if confidence >= 1.0, auto-creates alias and returns resolved; otherwise returns `{person_id: null, status: "unmapped"}`
- **Multiple matches for same alias**: Should not happen (application-level uniqueness); if it does, return the most recently created active alias

---

### Review Unmapped Aliases

- [ ] `p2` - **ID**: `cpt-ir-usecase-review-unmapped`

**Actor**: `cpt-ir-actor-operator`

**Preconditions**:
- Unmapped aliases exist with status `pending` or `in_review`
- Operator has access to the identity resolution API

**Main Flow**:
1. Operator calls `GET /unmapped?status=pending` to list unresolved aliases
2. For each unmapped alias, operator reviews the `suggested_person_id` (if any)
3. Operator calls `POST /unmapped/:id/resolve` with `person_id` to link alias to a person
4. System creates alias in `aliases` table and updates `unmapped` record with resolution details

**Postconditions**:
- Alias created in `aliases` table
- `unmapped` record updated: `status = 'resolved'`, `resolved_person_id`, `resolved_at`, `resolution_type = 'linked'`

**Alternative Flows**:
- **Operator creates new person**: Operator calls person domain API to create person, then links unmapped alias to new `person_id`; `resolution_type = 'new_person'`
- **Operator ignores alias**: Operator calls `POST /unmapped/:id/ignore`; `status = 'ignored'`, `resolution_type = 'ignored'`

---

### Merge Two Person Alias Sets

- [ ] `p3` - **ID**: `cpt-ir-usecase-merge`

**Actor**: `cpt-ir-actor-operator`

**Preconditions**:
- Two person records exist that the operator has determined represent the same individual
- Both persons have aliases in the `aliases` table

**Main Flow**:
1. Operator calls `POST /merge` with `source_person_id`, `target_person_id`, `reason`, `actor_person_id`
2. System snapshots current aliases for both persons → `snapshot_before`
3. System reassigns all aliases from `source_person_id` to `target_person_id`
4. System snapshots merged state → `snapshot_after`
5. System records `merge_audits` row with action `merge`
6. System runs ConflictDetector on `target_person_id`
7. System returns `{status: "merged", audit_id}`

**Postconditions**:
- All aliases previously owned by `source_person_id` now point to `target_person_id`
- Full audit record in `merge_audits` with before/after snapshots

**Alternative Flows**:
- **Circular merge detected**: System returns HTTP 409 `merge_conflict`
- **Conflict detected post-merge**: ConflictDetector creates `conflicts` record; merge proceeds but operator is alerted

---

### GDPR Alias Purge

- [ ] `p3` - **ID**: `cpt-ir-usecase-gdpr-purge`

**Actor**: `cpt-ir-actor-operator`

**Preconditions**:
- GDPR erasure request received for a specific `person_id`
- Person's aliases exist in the `aliases` table

**Main Flow**:
1. Operator calls `POST /purge` with `person_id` and `actor_person_id`
2. System copies all aliases for that person to `alias_gdpr_deleted` with `purged_at` and `purged_by_person_id`
3. System sets `is_deleted = 1` on all aliases for that person in `aliases` table
4. System returns confirmation with count of purged aliases

**Postconditions**:
- Aliases archived in `alias_gdpr_deleted`
- Aliases no longer resolvable via `POST /resolve`, Dictionary lookup, or direct query
- ClickHouse Dictionary refreshes within TTL (30-60s) to exclude purged aliases

**Alternative Flows**:
- **No aliases found for person**: System returns success with `count: 0`
- **Person has active merge audit**: System warns operator but proceeds (merges are alias-level, not person-level)

---

## 9. Acceptance Criteria

- [ ] Aliases seeded from HR Bronze data are resolvable via `POST /resolve` within Phase 1 deployment
- [ ] BootstrapJob processes 100K `identity_inputs` rows and creates correct aliases without duplicates
- [ ] >= 80% of aliases are auto-resolved (confidence >= 1.0) after BootstrapJob runs on typical connector data
- [ ] Unmapped aliases appear in operator queue with correct suggestions
- [ ] No fuzzy rule (Phase B3) produces an auto-linked alias under any input
- [ ] Merge + split round-trip preserves exact alias state (snapshot comparison passes)
- [ ] GDPR purge renders aliases unresolvable within 60 minutes
- [ ] Cross-tenant resolution returns empty for mismatched `insight_tenant_id`
- [ ] All mutating operations are idempotent (replay with same `Idempotency-Key` returns original result)

---

## 10. Dependencies

| Dependency | Description | Criticality |
|---|---|---|
| ClickHouse 24.x+ | Storage engine for all identity resolution tables; `generateUUIDv7()` support required | `p1` |
| Person domain (`persons` table) | Provides `person_id` targets for alias mapping; identity resolution does not create persons | `p1` |
| dbt models (Bronze → Silver) | Populate `identity_inputs` during connector sync transformations | `p1` |
| Argo Workflows | Orchestrates BootstrapJob scheduling and execution on Kind K8s | `p1` |
| Connector sync pipeline | Writes alias observations to `identity_inputs`; must conform to write contract | `p1` |
| Person domain (person creation API) | Operator needs to create new persons when linking unmapped aliases to new identities | `p2` |

---

## 11. Assumptions

- Person records are created by the person domain (dbt seed in Phase 1, API in later phases) before identity resolution links aliases to them. Identity resolution does not create persons.
- Connectors conform to the `identity_inputs` write contract and provide accurate `value_field_name` values.
- ClickHouse 24.x+ is available in all deployment environments with `generateUUIDv7()` support.
- The five alias types (`email`, `username`, `employee_id`, `display_name`, `platform_id`) cover all current connector identity signals. New types can be added as configuration without schema changes.
- HR source data (BambooHR, Workday) provides the most reliable identity anchors for initial seeding.
- Operator reviews the unmapped queue on a regular basis. Backlog alerts trigger if queue exceeds configured threshold (see NFR `unmapped_rate < 20%`).

---

## 12. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| ClickHouse lacks ACID transactions for merge/split (late phase — not yet implemented) | Partial state possible if operation fails mid-execution | See DESIGN §5 REC-IR-01: advisory locking + idempotent operations; retry-safe design |
| Connector writes malformed `identity_inputs` rows | BootstrapJob fails or creates incorrect aliases | Write contract validation at ingestion; malformed rows logged and skipped |
| Person domain unavailable during bootstrap | BootstrapJob cannot resolve new identities to person records | Route to `unmapped` queue; retry on next run when person domain is available |
| False-negative matching (too conservative) | Legitimate aliases stuck in unmapped queue; operator burden increases | Monitor unmapped rate; tune B2 rules for cross-system matching |
| ClickHouse ReplacingMergeTree dedup delay | Duplicate alias rows visible briefly before background merge | Application-level dedup check on read; FINAL keyword for critical queries |
| Scale: large organization with > 100K persons | Bootstrap throughput or alias lookup latency degrades | Benchmark at 100K+ scale; optimize ORDER BY keys and Dictionary layout |
| Domain boundary misunderstanding | Teams accidentally put person-domain logic in identity-resolution | Clear scope documentation (§4); code review enforcement of domain boundaries |
