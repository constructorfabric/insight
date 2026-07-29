# PRD — Presentation Layer

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
  - [5.1 Read-Only Safety (p1)](#51-read-only-safety-p1)
  - [5.2 Saved Queries (p1)](#52-saved-queries-p1)
  - [5.3 Tenant Isolation (p1)](#53-tenant-isolation-p1)
  - [5.4 Contract Stability (p2)](#54-contract-stability-p2)
  - [5.5 FE Loop (p2)](#55-fe-loop-p2)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Author and Run a Saved Query](#author-and-run-a-saved-query)
  - [Validate a Widget on a Preview Environment](#validate-a-widget-on-a-preview-environment)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Presentation Layer is the self-serve analytics surface of Insight: gold datasets, a query API, and a front-end (FE), evolving fast without ever endangering the source of truth. It is split from the Engineering layer along an explicit contract (epic #1803, Phase A "thin split"). The Engineering layer owns ingestion, silver facts, and identity artifacts and is read-only to presentation; the Presentation layer reads that contract and writes only its own `presentation` namespace.

This PRD covers Phase A: establish the layer boundary and make it safe, reusing the current analytics service. It ships a read-only query gate, a dedicated read-only role and an empty `presentation` namespace, a saved-query CRUD-and-run API, a server-injected tenant filter, and a stable query console with path-based preview environments for the FE loop.

### 1.2 Background / Problem Statement

Today gold, the query API, and metric semantics are entangled with ingestion in one analytics service. A broken or LLM-generated query can, in principle, reach the same database that holds the source of truth. There is no enforced boundary preventing a presentation-side write from altering engineering-owned data, and the tenant filter on reads is currently a no-op left over from the single-tenant MVP.

This blocks a fast, self-serve analytics loop. Analysts cannot safely author ad-hoc queries, and FE developers have no isolated place to build and validate new widgets. Any experiment risks the source data or leaks across tenants.

Phase A draws the contract and makes it safe by construction, so a new analytics slice needs no engineering change and no re-ingest. Metric semantics (the declarative registry and raw-to-derived compiler) are deliberately deferred to Phase B; this PRD does not encode choices that block them.

**Target users**: PMs and analysts authoring and running saved queries; FE developers building widgets on preview environments; the analytics service that hosts the query API and enforces the gate.

### 1.3 Goals (Business Outcomes)

| Goal | Success Criteria |
|------|------------|
| The source is safe | **Baseline**: presentation and engineering share one writable surface. **Target**: no presentation-side operation can write, alter, or drop engineering-owned data; worst case is damage to `presentation` scratch. **Timeframe**: end of Phase A. |
| Self-serve analytics without engineering change | **Baseline**: every new slice needs an engineering change or re-ingest. **Target**: an analyst authors, saves, and runs a new query with no deploy. **Timeframe**: end of Phase A. |
| Every read is tenant-scoped | **Baseline**: tenant filter is a no-op. **Target**: 100% of contract reads carry a server-injected tenant predicate the client cannot widen. **Timeframe**: end of Phase A. |
| Isolated FE authoring loop | **Baseline**: no isolated place to build widgets. **Target**: FE developers build and validate widgets on a shared read-only preview backend on synthetic data. **Timeframe**: end of Phase A. |

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Engineering layer | The contract producer: silver normalized facts (`class_*`, `fct_*`, `mtr_*`) and identity artifacts (`person.*`, `identity.*`). Read-only to presentation. |
| Presentation layer | Gold, query API, and FE that read the contract and write only the `presentation` namespace. |
| Contract | The read-only boundary the engineering layer exposes to presentation; additive-only and versioned. |
| `presentation` namespace | The ClickHouse database presentation owns for its own gold, saved-query results, and scratch. |
| Single-SELECT gate | A parse-and-reject check that accepts exactly one `SELECT`/`WITH` statement and rejects everything else. |
| `presentation_ro` role | A ClickHouse role with `SELECT` on the contract and `CREATE`/`INSERT` only in `presentation`. |
| Saved query | A stored `SELECT`/`WITH` over the contract with an id, name, and tenant, runnable read-only. |
| Preview environment | A path-based FE deployment (`/exp/<name>`) against a shared read-only synthetic backend. |
| Relabel, not migrate | Legacy gold stays read-only in the `insight` DB; only new gold is authored in `presentation`. |

## 2. Actors

### 2.1 Human Actors

#### PM / Analyst

**ID**: `cpt-presentation-actor-analyst`

**Role**: Authors, saves, and runs queries over the contract through the query console. Owns the correctness judgment on results.
**Needs**: To create and run ad-hoc queries safely, without an engineering change or re-ingest, scoped to their own tenant.

#### FE Developer

**ID**: `cpt-presentation-actor-fe-dev`

**Role**: Builds tier-3 bespoke widgets over the FE and validates them on preview environments before opening a PR.
**Needs**: An isolated place to run experimental FE builds against a stable, read-only backend without touching customer data.

### 2.2 System Actors

#### Analytics Service

**ID**: `cpt-presentation-actor-analytics-svc`

**Role**: Hosts the query API, enforces the single-SELECT gate, injects the tenant filter, and executes reads as the read-only role. The core presentation-layer runtime (Rust).

#### ClickHouse

**ID**: `cpt-presentation-actor-clickhouse`

**Role**: External storage engine holding both the contract (silver and identity databases, legacy gold in `insight`) and the `presentation` namespace. Enforces role grants.

#### Engineering / Ingestion Layer

**ID**: `cpt-presentation-actor-engineering`

**Role**: External upstream that produces the read-only contract (silver facts and identity artifacts) via ingestion and dbt. Changes are additive and versioned.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **No orchestrator in the platform**: there is no Argo/GitOps. Preview environment provisioning is manual `kubectl apply` / `helm upgrade`.
- **Single preview host**: one host serves all preview experiments, so there is one Entra redirect URI (Entra has no reliable wildcard redirect).
- **Contract is read-only to presentation**: presentation reads silver and identity databases and legacy gold in `insight`; it writes only `presentation`.

## 4. Scope

### 4.1 In Scope

- Read-only enforcement by construction: a query-path gate accepting exactly one `SELECT`/`WITH`, plus a read-only role.
- A `presentation` namespace for new gold, saved-query results, and scratch — new gold only, legacy gold relabelled not migrated.
- Saved-query CRUD and read-only run over the contract, with named parameters (`tenant` always injected, plus period).
- Server-injected flat tenant filter replacing the no-op on every contract read.
- Contract surface documentation and a contract version stamp.
- A stable query console (FE) and path-based preview environments with an authenticated return path.

### 4.2 Out of Scope

- Declarative metric registry, semantic raw-to-derived compiler, and FE metric rework (Phase B, #1974-#1978).
- Tenant subtree/hierarchy scoping and a row-policy backstop (deferred pending benchmark).
- Physical relocation of legacy gold from `insight` to `presentation` (deferred, #1979-#1981).
- Write-back from presentation to the contract (presentation is read-only for v1).
- **Safety-critical / life-critical behavior**: Not applicable — internal analytics platform, no physical actuation.
- **Accessibility, i18n, offline operation**: Not applicable — internal analytics platform with an authenticated, online, single-locale operator audience for Phase A.
- **Payments, PCI, HIPAA**: Not applicable — internal analytics platform, no payment or regulated-health data handled.

## 5. Functional Requirements

> **Testing strategy**: All requirements verified via automated tests (unit, integration, e2e) targeting 90%+ code coverage unless otherwise specified.

### 5.1 Read-Only Safety (p1)

#### Single-SELECT Query Gate

- [x] `p1` - **ID**: `cpt-presentation-fr-single-select-gate`

The system **MUST** accept exactly one read statement — a single `SELECT`/`WITH` query — on the public query path, and reject empty input, multiple statements, non-read statements (DDL/DML), and unparseable input. Acceptance is decided by parsing the statement, not by a textual prefix, so an equivalent parenthesized query is also one read statement. The gate **MUST** run on both query write (create/update) and run. (Shipped, #1962.)

**Rationale**: A broken or LLM-generated query must be unable to write, alter, or drop anything; the worst case is damage to `presentation` scratch.

**Actors**: `cpt-presentation-actor-analytics-svc`, `cpt-presentation-actor-analyst`

#### Read-Only Role

- [ ] `p1` - **ID**: `cpt-presentation-fr-read-only-role`

The system **MUST** execute contract reads under a dedicated `presentation_ro` role that has `SELECT` on the silver, identity, `person`, and legacy-gold (`insight`) databases and `CREATE`/`INSERT` only in `presentation`, with no `DROP`/`ALTER`/`TRUNCATE` anywhere. (#1963 provisions the role; it becomes the query-path identity once the analytics connection is wired to execute as it.)

**Rationale**: Read-only enforced by construction, not by convention.

**Actors**: `cpt-presentation-actor-analytics-svc`, `cpt-presentation-actor-clickhouse`

#### Presentation Namespace

- [ ] `p1` - **ID**: `cpt-presentation-fr-namespace`

The system **MUST** provide an empty `presentation` ClickHouse database for new presentation artifacts (new gold, saved-query results, scratch). Legacy gold **MUST** remain read-only in the `insight` database (relabel, not migrate). (#1964.)

**Rationale**: Presentation needs a place to write without touching engineering-owned data, without a disruptive physical migration.

**Actors**: `cpt-presentation-actor-analytics-svc`, `cpt-presentation-actor-clickhouse`

### 5.2 Saved Queries (p1)

#### Saved-Query CRUD and Run

- [ ] `p1` - **ID**: `cpt-presentation-fr-saved-query-crud`

The system **MUST** allow an analyst to create, list, fetch, update, delete, and run saved queries scoped to their tenant. Create and update **MUST** validate the SQL through the single-SELECT gate; run **MUST** execute read-only and return rows. (#1965.)

**Rationale**: A new analytics slice needs no engineering change and no re-ingest.

**Actors**: `cpt-presentation-actor-analyst`, `cpt-presentation-actor-analytics-svc`

#### Named Query Parameters

- [ ] `p1` - **ID**: `cpt-presentation-fr-query-params`

The system **MUST** support named query parameters, always injecting `tenant` from request context and supporting a `period` parameter. The `tenant` parameter **MUST NOT** be settable from client SQL. (#1966.)

**Rationale**: Consistent, safe parameterization; the tenant value is authoritative from context.

**Actors**: `cpt-presentation-actor-analyst`, `cpt-presentation-actor-analytics-svc`

### 5.3 Tenant Isolation (p1)

#### Server-Injected Tenant Filter

- [ ] `p1` - **ID**: `cpt-presentation-fr-tenant-filter`

The system **MUST** inject a literal tenant predicate (`insight_tenant_id = <ctx.tenant>`) server-side on every contract read, sourced from request context and not from client SQL. This **MUST** replace the current no-op filter. (#1967, coordinated with engineering #1829.)

**Rationale**: Every read is tenant-scoped; client SQL cannot widen it.

**Actors**: `cpt-presentation-actor-analytics-svc`, `cpt-presentation-actor-analyst`

### 5.4 Contract Stability (p2)

#### Contract Surface Documentation

- [ ] `p2` - **ID**: `cpt-presentation-fr-contract-surface-doc`

The system **MUST** document the contract surface — the silver and identity objects presentation may read — so additive-only evolution can be checked against it. (#1968.)

**Rationale**: The contract must be explicit for both layers to evolve it safely and additively.

**Actors**: `cpt-presentation-actor-engineering`, `cpt-presentation-actor-analytics-svc`

#### Contract Version Stamp

- [ ] `p2` - **ID**: `cpt-presentation-fr-contract-version-stamp`

The system **MUST** stamp a contract version so presentation can detect the contract surface it was built against. (#1969.)

**Rationale**: Additive-only changes are safer to reason about with an explicit version.

**Actors**: `cpt-presentation-actor-engineering`, `cpt-presentation-actor-analytics-svc`

### 5.5 FE Loop (p2)

#### Stable Query Console

- [ ] `p2` - **ID**: `cpt-presentation-fr-query-console`

The system **MUST** provide a single stable FE app (not per-branch stands) where an analyst authors a query (name plus SQL), gets an id, picks from their saved queries, runs it, and sees the result as a table or auto-chart. The console **MUST** consume only the saved-query API. (#1970.)

**Rationale**: Makes the saved-query API tangible and is query-management v0.

**Actors**: `cpt-presentation-actor-analyst`

#### Preview Environments

- [ ] `p2` - **ID**: `cpt-presentation-fr-preview-envs`

The system **MUST** serve many experimental FE builds under path-based addressing (`/exp/<name>`) on a single host, against one shared read-only backend on synthetic data (never customer production). Only the FE varies per experiment. (#1971, #1973.)

**Rationale**: FE developers need an isolated tier-3 authoring loop that cannot touch customer data.

**Actors**: `cpt-presentation-actor-fe-dev`

#### Preview Authentication Return Path

- [ ] `p2` - **ID**: `cpt-presentation-fr-preview-auth`

The system **MUST** authenticate preview environments through a single fixed callback using a Redis-backed opaque `state` return path, validated at store time (same-origin, `/exp/` allowlist), with nothing tamperable carried in the URL. (#1972.)

**Rationale**: One host means one Entra redirect URI; the return path must be safe with no per-experiment Entra change.

**Actors**: `cpt-presentation-actor-fe-dev`, `cpt-presentation-actor-analytics-svc`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Source Immutability from Presentation

- [ ] `p1` - **ID**: `cpt-presentation-nfr-source-immutability`

The system **MUST** guarantee that no presentation-side operation can write, alter, or drop engineering-owned data under any input, including malformed or adversarial SQL.

**Threshold**: 0 successful writes/alters/drops on contract objects from the presentation path in adversarial testing.

**Rationale**: The core guarantee of the split — the source is safe by construction.

#### Tenant Read Isolation

- [ ] `p1` - **ID**: `cpt-presentation-nfr-tenant-isolation`

Contract reads for tenant A **MUST NOT** return rows from tenant B, regardless of client SQL.

**Threshold**: 0 cross-tenant rows returned in isolation testing.

**Rationale**: Multi-tenant SaaS compliance requirement.

### 6.2 NFR Exclusions

- **High availability / clustering**: Presentation is not on a real-time serving path; ClickHouse cluster availability is managed at infrastructure level.
- **Query latency SLA**: Not specified for Phase A — ad-hoc analytical queries have no committed latency target yet; deferred to Phase B.
- **Encryption at rest**: Handled by ClickHouse infrastructure configuration.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Saved-Query API

- [ ] `p1` - **ID**: `cpt-presentation-interface-saved-query-api`

**Type**: REST API (HTTP/JSON)

**Stability**: unstable

**Description**: CRUD and read-only run over saved queries, tenant-scoped. The one new surface Phase A adds. Detailed endpoint contracts live in DESIGN.

**Breaking Change Policy**: Unstable in Phase A; contract hardens in a later phase.

### 7.2 External Integration Contracts

#### Read-Only Contract Consumption

- [ ] `p1` - **ID**: `cpt-presentation-contract-read-only-consumption`

**Direction**: required from external (presentation reads engineering-owned silver and identity objects)

**Protocol/Format**: ClickHouse `SELECT` over contract databases, executed as `presentation_ro`.

**Compatibility**: Contract changes are additive (new tables/columns), never rewrites; existing views keep working.

## 8. Use Cases

### Author and Run a Saved Query

- [ ] `p1` - **ID**: `cpt-presentation-usecase-author-run-query`

**Actor**: `cpt-presentation-actor-analyst`

**Preconditions**:
- The analyst is authenticated and has a tenant context.
- The `presentation` namespace and read-only role exist.

**Main Flow**:
1. Analyst submits a query (name plus SQL) through the console.
2. System validates the SQL through the single-SELECT gate and stores it, returning an id.
3. Analyst selects the saved query and runs it.
4. System injects the tenant filter, executes read-only as `presentation_ro`, and returns rows.

**Postconditions**:
- The saved query is stored, tenant-scoped, and runnable.
- Results are returned scoped to the analyst's tenant.

**Alternative Flows**:
- **SQL fails the gate**: System rejects create/update with a validation error; nothing is stored.

### Validate a Widget on a Preview Environment

- [ ] `p2` - **ID**: `cpt-presentation-usecase-preview-widget`

**Actor**: `cpt-presentation-actor-fe-dev`

**Preconditions**:
- A preview route object exists for the experiment (`/exp/<name>`).
- The shared read-only synthetic backend is available.

**Main Flow**:
1. FE developer applies the per-experiment bundle (FE image plus route object).
2. Developer opens `/exp/<name>/` and authenticates through the fixed callback.
3. System resolves the Redis-backed `state` return path and establishes the session.
4. Developer exercises the widget against the shared read-only synthetic backend.

**Postconditions**:
- The widget is validated against synthetic data with no access to customer production.

**Alternative Flows**:
- **`state` missing or expired**: System rejects the callback; the developer restarts login.

## 9. Acceptance Criteria

- [ ] No presentation-side operation can write, alter, or drop contract objects (adversarial SQL included).
- [ ] The single-SELECT gate rejects empty input, multiple statements, non-read (DDL/DML) statements, and unparseable input on write and run.
- [ ] An analyst can create, list, fetch, update, delete, and run a saved query with no engineering change.
- [ ] Every contract read carries a server-injected tenant predicate the client cannot widen.
- [ ] Cross-tenant reads return no rows.
- [ ] The `presentation` namespace exists and legacy gold remains read-only in `insight`.
- [ ] The query console runs a saved query and renders the result as a table or auto-chart.
- [ ] A preview environment serves an FE build under `/exp/<name>` on the shared read-only synthetic backend with an authenticated return path.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| ClickHouse | Stores the contract and the `presentation` namespace; enforces role grants | `p1` |
| Analytics service (Rust) | Hosts the query API, gate, and tenant-filter injection | `p1` |
| Engineering / ingestion layer | Produces the read-only contract (silver, identity) | `p1` |
| Engineering issue #1829 | Coordinated tenant-id retrofit for the flat tenant filter | `p1` |
| Redis | Backs the opaque `state` return path for preview auth | `p2` |
| Entra (OIDC) | Identity provider for the single preview host redirect URI | `p2` |

## 11. Assumptions

- The current analytics service is reused for Phase A; no new service is introduced.
- The contract evolves additively; existing views keep working.
- Presentation is strictly read-only for v1 (write-back is out of scope).
- Flat tenant isolation is sufficient for Phase A; subtree/hierarchy scoping is deferred to a benchmark.
- There is no orchestrator in the platform; preview provisioning is manual for now.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Gate bypass via crafted SQL | A write/alter reaches engineering-owned data | Enforce read-only by construction with the `presentation_ro` role in addition to the gate; adversarial test suite |
| No-op tenant filter left in place on a read path | Cross-tenant data leak | Inject the predicate in one shared place; cover with cross-tenant isolation tests |
| Tenant-id retrofit (#1829) lands out of sync | Filter targets a missing/renamed column | Coordinate the flat filter with engineering #1829 |
| Manual preview provisioning drifts or leaks | Stale experiments or config rewritten | One route object per experiment, merged by the controller; central config never rewritten |
| Preview `state` tampering | Open-redirect or session hijack on the return path | Validate `return_to` at store time (same-origin, `/exp/` allowlist); delete-on-read opaque `state` |
