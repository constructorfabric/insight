# PRD — Person Domain

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
  - [5.1 Golden Record Assembly (p1)](#51-golden-record-assembly-p1)
  - [5.2 Person Status Management (p1)](#52-person-status-management-p1)
  - [5.3 Person Conflict Detection & Resolution (p2)](#53-person-conflict-detection--resolution-p2)
  - [5.4 Person Availability Tracking (p2)](#54-person-availability-tracking-p2)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Golden Record Assembly from Source Update](#golden-record-assembly-from-source-update)
  - [Resolve Person-Attribute Conflict](#resolve-person-attribute-conflict)
  - [Ingest Person Availability](#ingest-person-availability)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

The Person domain owns the canonical person record — the single source of truth for who each person is and what their current attributes are. When multiple source systems (BambooHR, Active Directory, GitLab, Jira) provide overlapping and sometimes contradictory information about the same person, the Person domain assembles a golden record using configurable source-priority rules. It also tracks person availability (leave, capacity) and manages person lifecycle status.

### 1.2 Background / Problem Statement

Insight connects to 10+ external platforms, each contributing partial person data: BambooHR provides employee IDs and roles, Active Directory provides emails and departments, Git provides display names from commits. Before the Person domain, these attributes were scattered across source-specific Bronze tables with no unified view.

**Current state**: The original identity-resolution monolith handled both alias mapping and person attribute management. As part of the domain split, person records and golden record assembly are now a separate domain. The Identity Resolution domain resolves aliases to `person_id`; the Person domain owns the `persons` table that `person_id` points to.

**Key problems solved**:
- **No single source of truth**: Without golden record assembly, dashboards show inconsistent person attributes depending on which source was queried
- **Silent data conflicts**: When HR says `role=Engineer` and AD says `role=Platform Engineer`, the system must surface the disagreement rather than arbitrarily pick one
- **Stale person data**: Person attributes change (promotions, name changes, departures) and must be updated from the latest source contributions
- **Missing availability context**: Productivity metrics are meaningless without knowing when a person was on leave

**Target users**: Platform operators reviewing person data quality; analytics consumers relying on accurate person attributes; HR system administrators managing person lifecycle.

### 1.3 Goals (Business Outcomes)

| Goal | Success Criteria |
|---|---|
| Single source of truth for person attributes | **Baseline**: Attributes scattered across Bronze tables. **Target**: 100% of Gold analytics queries use `persons` table for person attributes. **Timeframe**: Within 1 sprint of Phase 1 deployment. |
| Source-priority golden record consistency | **Baseline**: No golden record. **Target**: Golden record rebuilt within 30 min of any source change; 100% deterministic given same inputs + priority config. **Timeframe**: From Phase 1 deployment. |
| Conflict visibility | **Baseline**: Conflicts silently ignored. **Target**: 100% of source disagreements surfaced in `person_conflicts`; operator reviews < 5% (conflicts that priority cannot resolve). **Timeframe**: Within 30 days of conflict detection deployment. |
| Person completeness tracking | **Baseline**: No completeness metric. **Target**: `completeness_score` accurately reflects non-empty canonical attributes for every person record. **Timeframe**: From Phase 1 deployment. |
| Availability-normalized analytics | **Baseline**: No leave data in analytics. **Target**: Person availability data integrated into Gold dashboards for capacity-adjusted metrics. **Timeframe**: From availability feature deployment. |

### 1.4 Glossary

| Term | Definition |
|---|---|
| Golden record | The single best-value view of a person assembled from all source contributions using source-priority rules |
| Source priority | Configurable ranking determining which source's value wins for each attribute (e.g., `manual` > `hr` > `git`) |
| Source contribution | A per-source snapshot of what that source says about a person's attributes |
| Person conflict | When two sources provide different values for the same canonical attribute and source priority cannot resolve the disagreement |
| Completeness score | Fraction of non-empty canonical attributes (0.0–1.0) for a person record |
| Canonical attribute | A standardized attribute name used in the golden record: `display_name`, `email`, `username`, `role`, `manager_person_id`, `org_unit_id`, `location` |
| Person status | Lifecycle state: `active`, `inactive`, `external`, `bot` |
| Person availability | Leave/capacity period: `vacation`, `sick_leave`, `parental_leave`, etc. |
| Bootstrap inputs | Shared table (IR domain) containing alias and person-attribute observations from connectors |

---

## 2. Actors

### 2.1 Human Actors

#### Operator

**ID**: `cpt-person-actor-operator`

**Role**: Reviews person-attribute conflicts, resolves disagreements between sources, manually overrides person attributes, and manages person lifecycle status. Typically a platform administrator or HR liaison.

**Needs**: A clear list of unresolved conflicts with both source values; ability to choose the correct value or override manually; visibility into completeness scores to identify data quality gaps; ability to deactivate or merge person records.

### 2.2 System Actors

#### GoldenRecordBuilder

**ID**: `cpt-person-actor-golden-record-builder`

**Role**: Assembles the best-value `persons` record from incoming person-attribute observations (read from `identity_inputs`), applying per-attribute source priority rules. Recomputes `completeness_score` on each rebuild. Invokes ConflictDetector when source priority cannot resolve a disagreement.

#### ConflictDetector

**ID**: `cpt-person-actor-conflict-detector`

**Role**: Compares attribute values from different sources for a given person. Creates `person_conflicts` records when values disagree and source priority alone cannot determine the correct value. Sets `conflict_status = 'needs_review'` on the person record.

#### HR Connector

**ID**: `cpt-person-actor-hr-connector`

**Role**: External system connector (BambooHR, Workday, LDAP) that writes person-attribute observations to the shared `identity_inputs` table. Provides the most authoritative source for employment data (role, department, manager, employee ID).

#### Analytics Pipeline

**ID**: `cpt-person-actor-analytics-pipeline`

**Role**: Downstream consumer (dbt models, dashboards) that reads from the `persons` table for cross-platform analytics. Depends on golden record fields being accurate, complete, and up-to-date. Also consumes `person_availability` for capacity-adjusted metrics.

---

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **Storage**: The Person-domain analytical tables (golden-record projection, `person_availability`, `person_conflicts`) reside in ClickHouse. The MariaDB-backed `persons` identity-attribute history table read by this domain is owned and maintained by the Identity-Resolution domain (see identity-resolution DESIGN §3.7 and ingestion [ADR-0006](../../ingestion/specs/ADR/0006-service-owned-migrations.md)).
- **Shared input**: Person-attribute observations originate from two sources — the `identity_inputs` ClickHouse table (connector-sourced) and the MariaDB `persons` history table (operator edits), both owned by the Identity Resolution domain.
- **SCD history**: Historical versions of person records are managed by dbt macros (SCD Type 2 / Type 3). This domain defines the current-state table; history schemas are out of scope.
- **Naming**: All tables and columns follow the shared glossary conventions.

---

## 4. Scope

### 4.1 In Scope

- Golden record assembly: per-attribute source priority, best-value selection, `*_source` tracking
- Person creation: dbt seed from HR Bronze (Phase 1), API-based creation (later phases)
- Person status management: `active`, `inactive`, `external`, `bot` lifecycle states
- Source contribution tracking: per-source attribute snapshots for golden record input
- Completeness scoring: fraction of non-empty canonical attributes
- Person-attribute conflict detection: flag disagreements between sources
- Person-attribute conflict resolution: operator workflow to choose correct values
- Person availability: leave/capacity period tracking from HR sources
- Tenant isolation on all person data

### 4.2 Out of Scope

- **Alias-to-person resolution**: Belongs to Identity Resolution domain — alias mapping, matching engine, unmapped alias queue
- **Org hierarchy**: `org_units` table, `person_assignments` table — belongs to Org-Chart domain
- **Connector implementation**: How connectors sync data from HR platforms
- **Permission / RBAC**: Access control and data visibility rules
- **SCD snapshot table schemas**: Managed by dbt macros, not this domain's application code
- **Metric aggregation**: Gold-layer dashboards and activity summaries

---

## 5. Functional Requirements

> **Testing strategy**: All requirements verified via automated tests (unit, integration, e2e) targeting 90%+ code coverage unless otherwise specified.

### 5.1 Golden Record Assembly (p1)

#### Create Person from HR Seed

- [ ] `p1` - **ID**: `cpt-person-fr-create-person-seed`

The system **MUST** create person records in the `persons` table from HR Bronze data via dbt seed models, populating `display_name`, `email`, `status`, and other available attributes.

**Rationale**: The person record is the foundation for all cross-domain references. Without it, the IR domain cannot link aliases and analytics cannot attribute activity.

**Actors**: `cpt-person-actor-hr-connector`

#### Track Source Contributions

- [ ] `p1` - **ID**: `cpt-person-fr-track-source-contributions`

The system **MUST** maintain a per-source attribute snapshot for each person. When a source provides updated attributes, the system **MUST** upsert the source contribution record with the new values and update `record_hash` for change detection.

**Rationale**: Golden record assembly requires knowing what each source says. Without per-source tracking, the system cannot apply source priority or detect conflicts.

**Actors**: `cpt-person-actor-golden-record-builder`

#### Assemble Golden Record with Source Priority

- [ ] `p1` - **ID**: `cpt-person-fr-golden-record-assembly`

The system **MUST** assemble the best-value golden record for each person by applying configurable per-attribute source priority rules. For each canonical attribute, the system **MUST** select the value from the highest-priority source that provides a non-empty value. The system **MUST** record which source provided each value in the corresponding `*_source` column.

**Rationale**: Multiple sources contribute overlapping data. Source priority ensures deterministic, auditable attribute selection — the same inputs always produce the same golden record.

**Actors**: `cpt-person-actor-golden-record-builder`

#### Compute Completeness Score

- [ ] `p1` - **ID**: `cpt-person-fr-completeness-score`

The system **MUST** compute `completeness_score` as the fraction of non-empty canonical attributes (display_name, email, username, role, manager_person_id, org_unit_id, location) out of 7 total. The score **MUST** be recomputed on every golden record rebuild.

**Rationale**: Completeness tracking enables operators to identify persons with missing data and prioritize data quality improvements.

**Actors**: `cpt-person-actor-golden-record-builder`, `cpt-person-actor-operator`

#### Change Detection for Source Contributions

- [ ] `p1` - **ID**: `cpt-person-fr-change-detection`

The system **MUST** detect changes in source contributions by comparing `record_hash` (SHA-256 of attribute values). When the hash is unchanged, the system **MUST NOT** trigger a golden record rebuild.

**Rationale**: Avoiding unnecessary rebuilds improves throughput when connectors re-sync unchanged data.

**Actors**: `cpt-person-actor-golden-record-builder`

#### Tenant Isolation

- [ ] `p1` - **ID**: `cpt-person-fr-tenant-isolation`

The system **MUST** isolate all person data by `insight_tenant_id`. Queries for tenant A **MUST NOT** return data from tenant B.

**Rationale**: Multi-tenant SaaS compliance requirement.

**Actors**: `cpt-person-actor-analytics-pipeline`, `cpt-person-actor-operator`

### 5.2 Person Status Management (p1)

#### Person Lifecycle Status

- [ ] `p1` - **ID**: `cpt-person-fr-status-management`

The system **MUST** support person lifecycle statuses: `active`, `inactive`, `external`, `bot`. Status changes **MUST** be persisted in the `persons` table. The system **SHOULD** infer initial status from HR source data (e.g., terminated employees → `inactive`).

**Rationale**: Dashboards must distinguish active employees from departed ones, external contractors from bots. Status drives filtering in all downstream analytics.

**Actors**: `cpt-person-actor-hr-connector`, `cpt-person-actor-operator`

#### Manual Attribute Override

- [ ] `p1` - **ID**: `cpt-person-fr-manual-override`

The system **MUST** allow operators to manually override any golden record attribute. Manual overrides **MUST** set the corresponding `*_source` column to `manual`, which has the highest priority and will persist across future golden record rebuilds.

**Rationale**: Automated assembly occasionally produces incorrect results (e.g., stale HR data). Operators need an escape hatch that survives rebuilds.

**Actors**: `cpt-person-actor-operator`

### 5.3 Person Conflict Detection & Resolution (p2)

#### Detect Person-Attribute Conflicts

- [ ] `p2` - **ID**: `cpt-person-fr-detect-conflicts`

The system **MUST** detect when two or more source contributions provide different values for the same canonical attribute and source priority cannot resolve the disagreement (e.g., two sources at the same priority level with different values). The system **MUST** create a conflict record with both source values.

**Rationale**: Silent data conflicts lead to incorrect analytics. Surfacing them enables operators to correct the data and improve source quality.

**Actors**: `cpt-person-actor-conflict-detector`

#### Flag Persons with Unresolved Conflicts

- [ ] `p2` - **ID**: `cpt-person-fr-flag-conflict-status`

When unresolved conflicts exist for a person, the system **MUST** set `persons.conflict_status = 'needs_review'`. When all conflicts are resolved or ignored, the system **MUST** set `conflict_status = 'clean'`.

**Rationale**: Operators need to find persons with data quality issues quickly. The `conflict_status` flag enables filtering without joining the conflicts table.

**Actors**: `cpt-person-actor-conflict-detector`, `cpt-person-actor-operator`

#### Operator Conflict Resolution

- [ ] `p2` - **ID**: `cpt-person-fr-resolve-conflicts`

The system **MUST** allow operators to resolve a person-attribute conflict by choosing the correct value. The resolution **MUST** update the golden record with the chosen value (as a `manual` override), mark the conflict as `resolved`, and record `resolved_by_person_id` and `resolved_at`.

**Rationale**: Not all conflicts can be resolved automatically. Operators with organizational knowledge must be able to pick the correct value.

**Actors**: `cpt-person-actor-operator`

#### Ignore Conflict

- [ ] `p2` - **ID**: `cpt-person-fr-ignore-conflict`

The system **MUST** allow operators to ignore a conflict (mark as `ignored`). Ignored conflicts **MUST NOT** contribute to `conflict_status = 'needs_review'`.

**Rationale**: Some conflicts are known acceptable differences (e.g., informal vs. formal name). Operators should be able to clear these from their queue.

**Actors**: `cpt-person-actor-operator`

#### List Person Conflicts

- [ ] `p2` - **ID**: `cpt-person-fr-list-conflicts`

The system **MUST** allow operators to list unresolved conflicts for a specific person and to list all persons with `conflict_status = 'needs_review'`.

**Rationale**: Operators need a queue-based workflow to efficiently review and resolve conflicts.

**Actors**: `cpt-person-actor-operator`

### 5.4 Person Availability Tracking (p2)

#### Ingest Availability Periods

- [ ] `p2` - **ID**: `cpt-person-fr-ingest-availability`

The system **MUST** accept person availability records (absence periods) with: `person_id`, `period_type` (vacation, sick_leave, parental_leave, public_holiday, unpaid_leave, other), `effective_from`, `effective_to` (half-open interval), and source identification.

**Rationale**: Productivity metrics are misleading without availability context. A person with zero commits during a vacation period is not underperforming.

**Actors**: `cpt-person-actor-hr-connector`

#### Query Availability for Date Range

- [ ] `p2` - **ID**: `cpt-person-fr-query-availability`

The system **MUST** allow querying a person's availability periods for a given date range. The system **MUST** support listing all persons with active availability periods on a given date.

**Rationale**: Gold-layer dashboards need availability data to compute capacity-adjusted metrics for any reporting period.

**Actors**: `cpt-person-actor-analytics-pipeline`

---

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Golden Record Freshness

- [ ] `p1` - **ID**: `cpt-person-nfr-golden-record-freshness`

The system **MUST** update the golden record within 30 minutes of a source contribution change arriving in `identity_inputs`.

**Threshold**: `persons.updated_at` within 30 min of corresponding `identity_inputs._synced_at`.

**Rationale**: Stale golden records cause analytics to show outdated person attributes (wrong role, wrong department), undermining trust in dashboards.

#### Tenant Data Isolation

- [ ] `p1` - **ID**: `cpt-person-nfr-tenant-isolation`

Person queries for tenant A **MUST NOT** return data from tenant B under any circumstances.

**Threshold**: 0 cross-tenant data leaks in penetration testing.

**Rationale**: Multi-tenant SaaS compliance requirement.

#### Completeness Score Accuracy

- [ ] `p1` - **ID**: `cpt-person-nfr-completeness-accuracy`

The `completeness_score` field **MUST** accurately reflect the current golden record attribute state. After any golden record rebuild, the score **MUST** equal `count(non-empty canonical attributes) / 7`.

**Threshold**: 100% accuracy verified by spot-checking 100 random person records.

**Rationale**: Inaccurate completeness scores would cause operators to miss data quality issues or chase phantom ones.

#### Person Query Latency

- [ ] `p1` - **ID**: `cpt-person-nfr-query-latency`

The system **MUST** return a single person record (GET /persons/:id) in < 50 ms at p99 under normal load.

**Threshold**: p99 latency < 50 ms at 500 req/s sustained.

**Rationale**: Person lookups are on the critical path for dashboards and cross-domain API calls.

### 6.2 NFR Exclusions

- **High availability / clustering**: Person domain is not on the real-time serving path for end users. ClickHouse cluster availability is managed at infrastructure level.
- **Real-time consistency**: Analytical consumers tolerate 30 min staleness for golden record updates. Real-time consistency is not required.
- **Encryption at rest**: Handled by ClickHouse infrastructure configuration, not by application-level encryption.

---

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Person REST API

- [ ] `p1` - **ID**: `cpt-person-interface-person-api`

**Type**: REST API (HTTP/JSON)

**Stability**: stable

**Description**: Primary interface for person CRUD, golden record queries, conflict management, and availability. Base path: `/api/persons/`.

**Breaking Change Policy**: Endpoint paths and response shapes are versioned; breaking changes require major version bump.

### 7.2 External Integration Contracts

#### Persons Table Cross-Domain Contract

- [ ] `p1` - **ID**: `cpt-person-contract-persons-table`

**Direction**: provided by library (person domain provides `persons.id`)

**Protocol/Format**: Logical FK — `aliases.person_id` (IR domain) and `person_assignments.person_id` (org-chart domain) reference `persons.id`

**Description**: The `persons` table is the canonical FK target for all cross-domain person references. The `id` column (UUID) is stable. Golden record fields (`display_name`, `email`, `role`, etc.) are available for denormalized reads.

**Compatibility**: The `id` UUID format and column name are stable. Adding new golden record columns is backward-compatible. Renaming or removing columns is breaking.

#### Identity Inputs Read Contract

- [ ] `p1` - **ID**: `cpt-person-contract-identity-inputs-read`

**Direction**: required from external (reads from IR domain's `identity_inputs` table)

**Protocol/Format**: ClickHouse SELECT

**Description**: The Person domain reads person-attribute observations from `identity_inputs` by filtering on `value_type` values that correspond to person attributes (`display_name`, `role`, `location`, `email`, `username`). The table schema is owned by the IR domain.

**Compatibility**: The Person domain depends on `identity_inputs` schema stability. Column additions are backward-compatible; column removals or renames require coordination with IR domain.

---

## 8. Use Cases

### Golden Record Assembly from Source Update

- [ ] `p1` - **ID**: `cpt-person-usecase-golden-record-assembly`

**Actor**: `cpt-person-actor-golden-record-builder`, `cpt-person-actor-hr-connector`

**Preconditions**:
- Person record exists in `persons` table
- HR connector has synced and written person-attribute observations to `identity_inputs`

**Main Flow**:
1. GoldenRecordBuilder reads new person-attribute observations from `identity_inputs` (filtered by person-attribute `value_type` values)
2. For each person with changes: compare incoming attributes against current `persons` row
3. Apply source priority per attribute; select highest-priority non-empty value
4. Write golden record fields to `persons` with corresponding `*_source` values
5. Recompute `completeness_score`
6. If no conflicts detected: set `conflict_status = 'clean'`

**Postconditions**:
- `persons` record reflects best-value attributes from all sources
- `*_source` columns track which source provided each value
- `completeness_score` is up-to-date

**Alternative Flows**:
- **Source priority tie (conflict)**: ConflictDetector creates `person_conflicts` record; `conflict_status = 'needs_review'`; golden record uses higher-priority source if available, otherwise retains existing value
- **First contribution for a new source**: golden record rebuilt with new source's attributes; `*_source` columns updated

---

### Resolve Person-Attribute Conflict

- [ ] `p2` - **ID**: `cpt-person-usecase-resolve-conflict`

**Actor**: `cpt-person-actor-operator`

**Preconditions**:
- Person has `conflict_status = 'needs_review'`
- Open conflict records exist in `person_conflicts`

**Main Flow**:
1. Operator queries `GET /persons?conflict_status=needs_review` to find persons with conflicts
2. Operator queries `GET /persons/:id/conflicts` to see conflict details
3. For each conflict: operator reviews `value_a` (from source A) and `value_b` (from source B)
4. Operator calls `POST /persons/:id/conflicts/:cid/resolve` with chosen value
5. System updates conflict: `status = 'resolved'`, `resolved_by_person_id`, `resolved_at`
6. System updates golden record: chosen attribute value set with `*_source = 'manual'`
7. If no more open conflicts: `conflict_status = 'clean'`

**Postconditions**:
- Conflict resolved; golden record updated with operator's choice
- Person `conflict_status` reflects remaining open conflicts

**Alternative Flows**:
- **Operator ignores conflict**: Calls ignore endpoint; conflict marked `ignored`; does not count toward `needs_review`
- **New source contribution arrives after resolution**: GoldenRecordBuilder respects `manual` override (highest priority); no new conflict created for that attribute unless manual is cleared

---

### Ingest Person Availability

- [ ] `p2` - **ID**: `cpt-person-usecase-ingest-availability`

**Actor**: `cpt-person-actor-hr-connector`

**Preconditions**:
- Person record exists in `persons` table
- HR connector has leave/capacity data

**Main Flow**:
1. HR connector writes availability records to `person_availability` table via insert
2. Each record includes: `person_id`, `period_type`, `effective_from`, `effective_to`, source identification
3. System stores the record

**Postconditions**:
- Availability period recorded and queryable for the person
- Gold dashboards can compute capacity-adjusted metrics

**Alternative Flows**:
- **Overlapping periods from same source**: System stores both; consumers handle overlap logic
- **Person not found**: Insert fails; connector logs error and retries after person creation

---

## 9. Acceptance Criteria

- [ ] Person records created from HR seed data include golden record fields with correct `*_source` values
- [ ] Golden record is deterministic: same source contributions + same priority config = same output
- [ ] Golden record rebuilds within 30 min of source contribution change
- [ ] `completeness_score` accurately reflects non-empty attribute count after every rebuild
- [ ] Source priority tie creates a `person_conflicts` record and sets `conflict_status = 'needs_review'`
- [ ] Operator can resolve a conflict and the golden record updates with `*_source = 'manual'`
- [ ] Manual overrides persist across subsequent golden record rebuilds (highest priority)
- [ ] Person availability periods are queryable by person and date range
- [ ] Cross-tenant person queries return empty for mismatched `insight_tenant_id`
- [ ] `GET /persons/:id` returns complete golden record in < 50 ms p99

---

## 10. Dependencies

| Dependency | Description | Criticality |
|---|---|---|
| ClickHouse 24.x+ | Storage engine for all person domain tables; `generateUUIDv7()` support | `p1` |
| IR domain (`identity_inputs` table) | Shared table providing person-attribute observations from connectors | `p1` |
| IR domain (`aliases` table) | References `persons.id` — IR domain is a consumer of person records | `p1` |
| dbt models | Seed `persons` from HR Bronze (Phase 1); manage SCD snapshots | `p1` |
| Org-chart domain (`person_assignments`) | References `persons.id` for assignment tracking | `p2` |
| Argo Workflows | Orchestrates GoldenRecordBuilder runs post-connector-sync | `p1` |

---

## 11. Assumptions

- Person records are created by dbt seed in Phase 1 (MVP). API-based person creation is a later-phase capability.
- The 7 canonical golden record attributes (display_name, email, username, role, manager_person_id, org_unit_id, location) cover all current analytical needs. New attributes can be added without schema changes to the priority mechanism.
- Source priority configuration is per-tenant. Each tenant may define its own attribute source priority rules.
- HR connectors (BambooHR, Workday) provide the most reliable person-attribute data and are ranked second only to manual overrides.
- The shared `identity_inputs` table schema is stable and owned by the IR domain. Person domain depends on column names and types remaining consistent.
- dbt manages SCD snapshot schemas independently; the Person domain does not need to know their structure.
- Operators review conflicts on a regular basis. Backlog alerts trigger if unresolved conflicts exceed a configured threshold.

---

## 12. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Source priority misconfiguration | Wrong source wins for attributes; golden records show stale or incorrect values | Default priority well-documented; operator manual override as escape hatch |
| High conflict volume overwhelms operators | Conflict queue grows unbounded; `needs_review` persons never resolved | Monitor conflict rate; tune priority rules to resolve more conflicts automatically; consider auto-resolve for low-impact attributes |
| `identity_inputs` schema change by IR domain | Person domain reads break silently; golden record assembly fails | Cross-domain contract (§7.2); schema changes require coordination |
| Completeness score misleading | Operators chase low scores for persons where missing attributes are expected (e.g., bots have no `manager_person_id`) | Consider per-status completeness targets; bots and external contractors may have different expected attribute sets |
| dbt seed timing vs. IR bootstrap timing | IR creates aliases before person records exist; aliases reference non-existent `person_id` | **Constraint**: Person dbt seed MUST complete before IR BootstrapJob runs. Enforced by Argo Workflow dependency ordering |
| ClickHouse ReplacingMergeTree dedup delay | Stale person records visible briefly after update | Application reads use FINAL keyword for critical queries; analytical queries tolerate brief staleness |
