# PRD — Org-Chart Domain

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
  - [5.1 Org Unit Hierarchy Management (p1)](#51-org-unit-hierarchy-management-p1)
  - [5.2 Person Assignment Tracking (p1)](#52-person-assignment-tracking-p1)
  - [5.3 Re-Org and Temporal Operations (p2)](#53-re-org-and-temporal-operations-p2)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
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

---

## 1. Overview

### 1.1 Purpose

The Org-Chart domain manages the organizational hierarchy — the tree of departments, teams, and divisions — and the temporal assignments of persons to those organizational units. It enables analytics to answer "which team did this person belong to when they made that commit?" by providing time-aware org structure and assignment data.

### 1.2 Background / Problem Statement

Insight's analytics capabilities depend on correct team attribution: commits, issues, and messages must be attributed to the correct department and team at the time of the event. Without temporal org data, a person who transferred from Engineering to Product in January would have all their 2025 Engineering commits incorrectly attributed to Product.

**Current state**: The org hierarchy tables (`org_units`, `person_assignments`) were previously embedded in the identity-resolution monolith. As part of the domain split, they are now a standalone domain with clear boundaries. The Person domain owns person records; the Org-Chart domain owns the org structure and person-to-org assignments.

**Key problems solved**:
- **Incorrect historical team attribution**: Without temporal assignments, analytics use current org state for all historical events, producing incorrect team velocity and resource metrics
- **Missing re-org handling**: Department renames, team restructuring, and manager changes need proper close-and-open semantics to preserve history
- **No unified org hierarchy**: Connector-specific department strings (e.g., "Engineering" from BambooHR, "ENG" from AD) need a canonical hierarchy with parent-child relationships
- **Limited query capabilities**: Analytics need subtree queries ("everyone in Engineering and its sub-teams"), level queries ("all department heads"), and point-in-time queries

**Target users**: Platform operators managing org structure; analytics consumers relying on team attribution; HR system administrators maintaining the org hierarchy.

### 1.3 Goals (Business Outcomes)

| Goal | Success Criteria |
|---|---|
| Correct historical team attribution | **Baseline**: All events attributed to current team. **Target**: 100% of Gold analytics use temporal assignments for event-date attribution. **Timeframe**: Within 1 sprint of deployment. |
| Re-org handling without data loss | **Baseline**: Re-orgs overwrite history. **Target**: 100% of re-orgs preserve prior state via close-and-insert. **Timeframe**: From deployment. |
| Unified org hierarchy | **Baseline**: Flat department strings from HR. **Target**: Tree-structured org units with parent-child relationships and materialized paths for all active departments. **Timeframe**: Within 30 days of initial load. |
| Point-in-time query support | **Baseline**: No temporal queries. **Target**: Any historical date query returns correct org structure and assignments. **Timeframe**: From deployment. |

### 1.4 Glossary

| Term | Definition |
|---|---|
| Org unit | A node in the organizational hierarchy: department, team, division, cost center |
| Person assignment | A temporal link between a person and an org unit, role, team, manager, or other organizational dimension |
| Materialized path | A string column storing the full hierarchy path (e.g., `/company/engineering/platform`) for efficient subtree queries |
| Half-open interval | Temporal range `[effective_from, effective_to)` — start is inclusive, end is exclusive |
| Re-org | An organizational restructuring: renaming, moving, or merging org units |
| Assignment type | The dimension of an assignment: `org_unit`, `role`, `department`, `team`, `manager`, `project`, `location`, `cost_center` |
| Legacy flat-string type | Assignment types (`department`, `team`) that use a string value instead of an `org_unit_id` FK — used before org hierarchy is configured |
| SCD Type 2 | Slowly Changing Dimension pattern: close old row, insert new row to preserve history |
| Sentinel date | `'1970-01-01'` used as the zero value for `effective_to` in ClickHouse (meaning "current / open-ended") |

---

## 2. Actors

### 2.1 Human Actors

#### Operator

**ID**: `cpt-orgchart-actor-operator`

**Role**: Manages the org unit hierarchy: creates departments and teams, handles re-orgs (renames, parent changes, merges), assigns persons to org units, and resolves organizational data quality issues. Typically an HR liaison or platform administrator.

**Needs**: Ability to create and restructure org units without losing history; visibility into current and historical assignments; tools to assign and transfer persons between org units.

### 2.2 System Actors

#### HR Connector

**ID**: `cpt-orgchart-actor-hr-connector`

**Role**: External system connector (BambooHR, Workday, LDAP) that provides organizational structure data — department names, hierarchy relationships, and person-to-department mappings. Writes org data via dbt models or the shared `identity_inputs` table.

#### Analytics Pipeline

**ID**: `cpt-orgchart-actor-analytics-pipeline`

**Role**: Downstream consumer (dbt Gold models, dashboards) that joins events (commits, issues, messages) against `person_assignments` to attribute activity to the correct org unit at the time of the event. Also queries org hierarchy for subtree aggregations (team velocity, department headcount).

---

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **Storage**: All org-chart domain tables reside in ClickHouse. No separate RDBMS.
- **Temporal model**: All temporal ranges use `[effective_from, effective_to)` half-open intervals with `Date` type. `BETWEEN` prohibited on temporal columns. Sentinel date `'1970-01-01'` means "current / open-ended".
- **SCD history**: SCD Type 2 snapshots for `org_units` and `person_assignments` are managed by dbt macros. This domain defines the source table schemas; dbt owns the derived snapshot schemas.
- **Naming**: All tables and columns follow PR #55 glossary conventions.

---

## 4. Scope

### 4.1 In Scope

- Org unit hierarchy management: create, update, deactivate org units with parent-child relationships
- Materialized path and depth computation for efficient hierarchy queries
- Person-to-org assignment with temporal tracking (half-open intervals)
- Multiple concurrent assignment types per person (org_unit, role, manager, team, etc.)
- Transfer handling: close current assignment, open new assignment (SCD2 close-and-insert)
- Re-org handling: rename, move, or merge org units with history preservation
- Legacy flat-string assignment types (`department`, `team`) for bootstrap before hierarchy is configured
- Point-in-time queries: "who was in department X on date Y?"
- Subtree queries: "everyone in Engineering and its sub-teams"
- Tenant isolation on all org data

### 4.2 Out of Scope

- **Person records**: `persons` table, golden record assembly — belongs to Person domain
- **Alias resolution**: Alias-to-person mapping — belongs to Identity Resolution domain
- **SCD snapshot table schemas**: `org_units_snapshot`, `person_assignments_snapshot` — managed by dbt macros
- **Connector implementation**: How HR connectors sync org data from external systems
- **Permission / RBAC**: Access control and data visibility rules
- **Metric aggregation**: Gold-layer team velocity, department headcount dashboards

---

## 5. Functional Requirements

> **Testing strategy**: All requirements verified via automated tests (unit, integration, e2e) targeting 90%+ code coverage unless otherwise specified.

### 5.1 Org Unit Hierarchy Management (p1)

#### Create Org Unit

- [ ] `p1` - **ID**: `cpt-orgchart-fr-create-org-unit`

The system **MUST** allow creating an org unit with `name`, `code`, and `parent_id`. The system **MUST** automatically compute `path` (materialized hierarchy path) and `depth` from the parent chain. Root org units **MUST** have `parent_id` set to zero UUID and `depth = 0`.

**Rationale**: The org hierarchy is the foundation for all team attribution. Without it, there is no structure to assign persons to.

**Actors**: `cpt-orgchart-actor-operator`, `cpt-orgchart-actor-hr-connector`

#### Update Org Unit Attributes

- [ ] `p1` - **ID**: `cpt-orgchart-fr-update-org-unit`

The system **MUST** allow updating an org unit's `name` and `code`. Attribute updates **MUST** close the current version (set `effective_to`) and insert a new version with updated attributes and a new `effective_from`, preserving history.

**Rationale**: Department renames happen regularly (e.g., "Engineering" → "Product Engineering"). History must be preserved for correct attribution of events that occurred under the old name.

**Actors**: `cpt-orgchart-actor-operator`

#### Deactivate Org Unit

- [ ] `p1` - **ID**: `cpt-orgchart-fr-deactivate-org-unit`

The system **MUST** allow deactivating an org unit by setting `effective_to` to the deactivation date. The system **SHOULD** warn if active person assignments still reference the org unit being deactivated.

**Rationale**: Departments are disbanded during restructuring. Deactivation must preserve the unit's history for past event attribution while preventing new assignments.

**Actors**: `cpt-orgchart-actor-operator`

#### Query Org Hierarchy

- [ ] `p1` - **ID**: `cpt-orgchart-fr-query-hierarchy`

The system **MUST** support querying the org hierarchy: list all org units for a tenant, get subtree for an org unit (all descendants), get ancestors for an org unit, and filter by depth level. All hierarchy queries **MUST** filter to currently active org units by default.

**Rationale**: Analytics and operators need to navigate the org structure efficiently — subtree queries for team aggregation, ancestor queries for reporting chains.

**Actors**: `cpt-orgchart-actor-analytics-pipeline`, `cpt-orgchart-actor-operator`

#### Tenant Isolation

- [ ] `p1` - **ID**: `cpt-orgchart-fr-tenant-isolation`

The system **MUST** isolate all org data by `insight_tenant_id`. Queries for tenant A **MUST NOT** return org units or assignments from tenant B.

**Rationale**: Multi-tenant SaaS compliance requirement.

**Actors**: `cpt-orgchart-actor-analytics-pipeline`, `cpt-orgchart-actor-operator`

### 5.2 Person Assignment Tracking (p1)

#### Create Person Assignment

- [ ] `p1` - **ID**: `cpt-orgchart-fr-create-assignment`

The system **MUST** allow creating a temporal assignment linking a `person_id` to an org unit, role, team, or other dimension. Each assignment **MUST** include `assignment_type`, `effective_from`, and either `org_unit_id` (for org_unit type) or `assignment_value` (for legacy flat-string types).

**Rationale**: This is the core data model — linking persons to org units over time is what enables team attribution in analytics.

**Actors**: `cpt-orgchart-actor-hr-connector`, `cpt-orgchart-actor-operator`

#### Transfer Person (Close and Open Assignment)

- [ ] `p1` - **ID**: `cpt-orgchart-fr-transfer`

When a person transfers to a new org unit (or role, team, etc.), the system **MUST** close the current assignment of the same type (set `effective_to` to the transfer date) and create a new assignment with `effective_from` equal to the transfer date. At no point **MUST** there be two active assignments of the same type for the same person.

**Rationale**: Transfers are the most common org change. The close-and-open pattern ensures exactly one active assignment per type at any date, enabling correct temporal analytics.

**Actors**: `cpt-orgchart-actor-hr-connector`, `cpt-orgchart-actor-operator`

#### Multiple Concurrent Assignment Types

- [ ] `p1` - **ID**: `cpt-orgchart-fr-multi-assignment`

The system **MUST** support multiple concurrent assignments of different types for the same person. A person **MUST** be able to have an active `org_unit` assignment, an active `role` assignment, an active `manager` assignment, and an active `project` assignment simultaneously.

**Rationale**: Organizational structure is multi-dimensional. A person belongs to a department, has a role, reports to a manager, and may work on a project — all independently tracked.

**Actors**: `cpt-orgchart-actor-hr-connector`

#### Query Current Assignments for a Person

- [ ] `p1` - **ID**: `cpt-orgchart-fr-query-current-assignments`

The system **MUST** allow querying all current (active) assignments for a given person, filtered by `effective_to` sentinel value.

**Rationale**: Dashboards and person profile pages need to show current org unit, role, and team without specifying a date.

**Actors**: `cpt-orgchart-actor-analytics-pipeline`, `cpt-orgchart-actor-operator`

#### Point-in-Time Assignment Query

- [ ] `p1` - **ID**: `cpt-orgchart-fr-point-in-time-query`

The system **MUST** support querying assignments for a person at a specific historical date using half-open interval logic: `effective_from <= target_date AND (effective_to = sentinel OR effective_to > target_date)`.

**Rationale**: Gold analytics join events (commits, issues) against assignments by event date. Incorrect temporal logic produces wrong team attribution.

**Actors**: `cpt-orgchart-actor-analytics-pipeline`

#### Support Legacy Flat-String Assignment Types

- [ ] `p1` - **ID**: `cpt-orgchart-fr-legacy-flat-string`

The system **MUST** support assignment types `department` and `team` with string values in `assignment_value`, without requiring an `org_unit_id` FK. These **SHOULD** be used as a bootstrap path before the org hierarchy is configured.

**Rationale**: Not all deployments have a pre-configured org hierarchy on day one. HR connectors provide department/team as strings; these must be stored until the hierarchy is built.

**Actors**: `cpt-orgchart-actor-hr-connector`

### 5.3 Re-Org and Temporal Operations (p2)

#### Re-Org: Change Org Unit Parent

- [ ] `p2` - **ID**: `cpt-orgchart-fr-reorg-parent`

The system **MUST** allow changing an org unit's parent (moving it in the hierarchy). The operation **MUST** close the current version, insert a new version with the updated parent, and recompute `path` and `depth` for the moved org unit and all its descendants.

**Rationale**: Organizational restructuring (e.g., moving Platform team from Engineering to Product) must preserve history and update hierarchy paths for correct subtree queries.

**Actors**: `cpt-orgchart-actor-operator`

#### Query Assignment History

- [ ] `p2` - **ID**: `cpt-orgchart-fr-assignment-history`

The system **MUST** support querying the full assignment history for a person, showing all current and closed assignments ordered by `effective_from`.

**Rationale**: Operators need to see a person's org history for auditing, onboarding context, and re-org planning.

**Actors**: `cpt-orgchart-actor-operator`

#### Bulk Load Org Hierarchy from HR

- [ ] `p2` - **ID**: `cpt-orgchart-fr-bulk-load`

The system **MUST** support bulk loading org units and person assignments from HR connector data via dbt models. The bulk load **MUST** be idempotent — re-running on unchanged data **MUST NOT** create duplicate org units or assignments.

**Rationale**: Initial org hierarchy setup comes from HR systems with hundreds of departments and thousands of assignments. Manual creation is not feasible.

**Actors**: `cpt-orgchart-actor-hr-connector`

---

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Hierarchy Query Latency

- [ ] `p1` - **ID**: `cpt-orgchart-nfr-hierarchy-query-latency`

The system **MUST** return org hierarchy queries (subtree, ancestors, level) in < 100 ms at p99 for hierarchies with up to 1000 org units.

**Threshold**: p99 latency < 100 ms at 100 req/s for subtree queries on 1000-node hierarchy.

**Rationale**: Hierarchy queries are on the critical path for dashboard rendering and assignment validation.

#### Temporal Query Correctness

- [ ] `p1` - **ID**: `cpt-orgchart-nfr-temporal-correctness`

The system **MUST** guarantee exactly one active assignment per `(person_id, assignment_type)` at any point in time. No double-attribution on boundary dates.

**Threshold**: 0 violations found in temporal consistency check across all tenants (verified by scheduled audit query).

**Rationale**: Double-attribution produces inflated team metrics and incorrect per-person analytics.

#### Tenant Data Isolation

- [ ] `p1` - **ID**: `cpt-orgchart-nfr-tenant-isolation`

Org queries for tenant A **MUST NOT** return data from tenant B under any circumstances.

**Threshold**: 0 cross-tenant data leaks in penetration testing.

**Rationale**: Multi-tenant SaaS compliance requirement.

### 6.2 NFR Exclusions

- **High availability / clustering**: Org-chart is not on the real-time serving path. ClickHouse cluster availability managed at infrastructure level.
- **Sub-second consistency**: Analytical consumers tolerate staleness during dbt model runs. Real-time consistency not required.
- **Encryption at rest**: Handled by ClickHouse infrastructure configuration.

---

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Org-Chart REST API

- [ ] `p1` - **ID**: `cpt-orgchart-interface-orgchart-api`

**Type**: REST API (HTTP/JSON)

**Stability**: stable

**Description**: Primary interface for org unit management, person assignment tracking, hierarchy traversal, and temporal queries. Base path: `/api/org-chart/`.

**Breaking Change Policy**: Endpoint paths and response shapes are versioned; breaking changes require major version bump.

### 7.2 External Integration Contracts

#### Person Domain Cross-Reference Contract

- [ ] `p1` - **ID**: `cpt-orgchart-contract-person-domain`

**Direction**: required from external (org-chart references `persons.id`)

**Protocol/Format**: Logical FK — `person_assignments.person_id` references `persons.id` (Person domain)

**Description**: The Org-Chart domain creates assignments for persons owned by the Person domain. The `person_id` FK must reference a valid person record. Person creation is the Person domain's responsibility.

**Compatibility**: Depends on `persons.id` UUID format remaining stable.

#### Org Units Provided Contract

- [ ] `p1` - **ID**: `cpt-orgchart-contract-org-units-provided`

**Direction**: provided by library (org-chart provides `org_units.id`)

**Protocol/Format**: Logical FK — `persons.org_unit_id` (Person domain golden record) references `org_units.id`

**Description**: The `org_units` table provides the canonical org hierarchy. The Person domain's golden record field `org_unit_id` references `org_units.id`. Analytics Gold models join against `org_units` for team attribution.

**Compatibility**: The `id` UUID format and `path` column are stable. Adding columns is backward-compatible.

---

## 8. Use Cases

#### Load Org Hierarchy from HR

- [ ] `p1` - **ID**: `cpt-orgchart-usecase-load-hierarchy`

**Actor**: `cpt-orgchart-actor-hr-connector`

**Preconditions**:
- HR connector has synced department/team structure data
- dbt models are configured to transform HR Bronze data into org units

**Main Flow**:
1. dbt model reads HR Bronze data (BambooHR departments, team structures)
2. For each department: create or update org unit with name, code, parent relationship
3. Compute materialized path and depth for each org unit
4. For each person-department mapping: create person assignment with `assignment_type = 'org_unit'`

**Postconditions**:
- Org units exist with correct parent-child relationships and materialized paths
- Person assignments link persons to their org units with `effective_from` dates

**Alternative Flows**:
- **Org unit already exists (idempotent)**: Update attributes if changed; skip if unchanged
- **Parent org unit not yet created**: Process parent first (topological sort) or defer child creation

---

#### Transfer Person Between Teams

- [ ] `p1` - **ID**: `cpt-orgchart-usecase-transfer`

**Actor**: `cpt-orgchart-actor-operator`, `cpt-orgchart-actor-hr-connector`

**Preconditions**:
- Person has an active `org_unit` assignment
- Target org unit exists and is active

**Main Flow**:
1. System receives transfer request: `person_id`, new `org_unit_id`, `effective_from` date
2. System finds current `org_unit` assignment where `effective_to = sentinel`
3. System closes current assignment: set `effective_to = transfer_date`
4. System creates new assignment: `org_unit_id = new_unit`, `effective_from = transfer_date`, `effective_to = sentinel`

**Postconditions**:
- Old assignment closed with `effective_to = transfer_date`
- New assignment active with `effective_from = transfer_date`
- Analytics queries for dates before transfer return old org unit; after transfer return new org unit

**Alternative Flows**:
- **No current assignment exists**: Create new assignment directly (first assignment for this person)
- **Transfer date is in the past**: System creates both close and open records with historical dates; existing analytics results may change on next Gold rebuild

---

#### Query Point-in-Time Team Composition

- [ ] `p2` - **ID**: `cpt-orgchart-usecase-point-in-time`

**Actor**: `cpt-orgchart-actor-analytics-pipeline`

**Preconditions**:
- Org units and person assignments populated
- Target date specified

**Main Flow**:
1. Analytics query specifies: org_unit (or subtree), target_date
2. System queries person_assignments: `assignment_type = 'org_unit'` AND `org_unit_id IN (subtree)` AND `effective_from <= target_date` AND `(effective_to = sentinel OR effective_to > target_date)`
3. System returns list of persons assigned to the org unit(s) on the target date

**Postconditions**:
- Caller receives correct team composition for the specified date

**Alternative Flows**:
- **No assignments for target date**: Returns empty list
- **Org unit was renamed on target date**: Query uses `org_units` effective dates to find correct org unit version

---

#### Handle Re-Org (Move Team)

- [ ] `p2` - **ID**: `cpt-orgchart-usecase-reorg`

**Actor**: `cpt-orgchart-actor-operator`

**Preconditions**:
- Source org unit exists and is active
- New parent org unit exists and is active
- No circular parent-child relationship would result

**Main Flow**:
1. Operator specifies: org_unit to move, new parent, effective_date
2. System closes current org_unit version (set `effective_to = effective_date`)
3. System inserts new version: same `id`, new `parent_id`, recomputed `path` and `depth`
4. System recomputes `path` and `depth` for all descendants of the moved org unit
5. System returns count of affected org units

**Postconditions**:
- Org unit and all descendants have updated paths reflecting new parent
- History preserved: queries for dates before re-org return old hierarchy structure
- Person assignments are NOT changed (they reference `org_unit_id`, which is stable across re-orgs)

**Alternative Flows**:
- **Circular dependency detected**: System rejects the operation
- **Org unit has no descendants**: Only the moved unit is updated

---

## 9. Acceptance Criteria

- [ ] Org units created with correct parent-child relationships, computed paths, and depth values
- [ ] Person transfers produce exactly one active assignment per type at any date (no double-attribution)
- [ ] Point-in-time queries return correct org unit for any historical date
- [ ] Re-org operations preserve history: old hierarchy queryable for dates before the re-org
- [ ] Subtree queries return all descendants of an org unit via materialized path
- [ ] Legacy flat-string types (`department`, `team`) stored without requiring `org_unit_id`
- [ ] Bulk load from HR data is idempotent — re-running produces no duplicates
- [ ] Cross-tenant org queries return empty for mismatched `insight_tenant_id`
- [ ] Hierarchy queries complete in < 100 ms p99 on 1000-node hierarchies

---

## 10. Dependencies

| Dependency | Description | Criticality |
|---|---|---|
| ClickHouse 24.x+ | Storage engine for all org-chart tables; `generateUUIDv7()` support | `p1` |
| Person domain (`persons` table) | Provides `person_id` targets for assignments; org-chart does not create persons | `p1` |
| dbt models | Seed/incremental load of org units and assignments from HR Bronze data | `p1` |
| Argo Workflows | Orchestrates dbt model runs post-connector-sync | `p1` |
| IR domain (`identity_inputs`) | Optional alternative ingestion path for org data from connectors | `p2` |

---

## 11. Assumptions

- Org hierarchy data is available from HR connectors (BambooHR, Workday) with department/team names and parent-child relationships.
- The materialized path convention (`/company/engineering/platform`) is sufficient for all hierarchy query patterns. Recursive CTEs are not needed.
- A single `person_id` has at most one active assignment per `assignment_type` at any point in time.
- Legacy flat-string assignment types (`department`, `team`) are retained indefinitely for deployments without an org hierarchy. Deployments with a configured hierarchy SHOULD use `org_unit` type. No mandatory migration planned.
- dbt manages SCD snapshot table schemas independently; the Org-Chart domain does not need to know their structure.
- Re-orgs are infrequent (monthly or quarterly at most) and can tolerate operator-initiated workflows.

---

## 12. Risks

| Risk | Impact | Mitigation |
|---|---|---|
| HR connector provides flat department names without parent-child relationships | Hierarchy cannot be built automatically; all org units appear at root level | Support legacy flat-string types as fallback; build hierarchy manually or from secondary source |
| Re-org recomputes paths for large subtrees | Performance impact if hundreds of org units need path recomputation | Batch path recomputation; schedule during low-traffic windows |
| Temporal boundary edge cases | Off-by-one errors on `effective_from`/`effective_to` boundaries produce double or missing attribution | Enforce half-open interval semantics; `BETWEEN` prohibited; extensive boundary-date test suite |
| Two parallel assignment models (flat-string + org_unit) | Different queries needed depending on type; potential confusion | Clear documentation per type; API abstracts both models behind unified interface |
| Person domain creates person after org-chart tries to assign | Assignment references non-existent `person_id`; insert may succeed in ClickHouse (no FK enforcement) | Ensure dbt runs person seed before assignment load; validate person_id existence at application level |
| Circular parent-child reference in org_units | Path computation loops infinitely; hierarchy queries break | Validate no cycles on parent change; reject circular references at application level |
