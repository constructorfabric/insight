# PRD — Semantic Layer

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
  - [5.1 Definitions and Compiler (p1)](#51-definitions-and-compiler-p1)
  - [5.2 Time and Scope (p1)](#52-time-and-scope-p1)
  - [5.3 Serving and Authoring (p2)](#53-serving-and-authoring-p2)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Author a Measure and a Metric](#author-a-measure-and-a-metric)
  - [Register a Custom Dataset](#register-a-custom-dataset)
  - [Promote a Definition to a Dashboard](#promote-a-definition-to-a-dashboard)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Semantic Layer is the system through which every analytical value in Insight is defined, validated, computed, and served. Definitions — datasets, measures, metrics, charts, dashboards — are data, not code: a row in a definition store that authorized users can eventually edit at runtime through structured editors. A single server-owned compiler turns each definition plus a request into warehouse SQL; storage, caching, and materialization are private implementation details behind the definition contract.

This PRD covers Phase B of the presentation-layer split (epic constructorfabric/insight#1803), after Phase A drew the read-only, tenant-scoped boundary. It formalizes definitions-as-data, one compiler over datasets, a gated custom-dataset SQL layer, capability derived from definitions rather than stored rows, server-owned semantics with query-time grain and a tenant reporting timezone, a discovery API, runtime editing, and — critically — the server-injected scopes (tenant, org-scope entity visibility, cohort isolation) the client can never widen. The full design rationale lives in [REFERENCE.md](./REFERENCE.md); the migration sequence in [IMPLEMENTATION.md](./IMPLEMENTATION.md); the adoption review and open items in [FINDINGS.md](./FINDINGS.md).

### 1.2 Background / Problem Statement

Today metric meaning is authored twice and lives in two places at once: as dbt SQL in gold observation models and again as Rust constants for the definition store, then pre-computed to observation rows at a baked day grain. Capability is inferred by probing observed rows, so an empty tenant reports no capability and partial ingestion can silently mutate the served contract. Adding one metric takes three deploys and two hand-synchronized restatements, and the chart is frontend code.

This blocks self-serve analytics and safe evolution. There is no single executor, so two interpreters of one definition can diverge on null handling, time zones, and deduplication. There is no runtime authoring, no query-time grain, and no place for expressive analytics (joins, windows, funnels) that is safe by construction. Authorization scopes beyond tenancy — who may see which people, which cohorts — are enforced at the request boundary rather than uniformly at the compiler, so a new read path can miss them.

Phase B replaces this with one compiler over datasets and definitions-as-data. Meaning exists in exactly one place; capability is a projection of definitions; every query the compiler emits carries the same injected scopes. Expressiveness that the structured layers deliberately cannot reach lands in a single gated SQL layer (custom datasets) with dataset-sized blast radius, never in a loosened editor.

**Target users**: admins authoring measures and metrics; dataset authors registering custom SQL datasets under a gated role; analysts and viewers reading served values within their scope; the compiler service that owns semantics; the identity service that owns org-chart visibility.

### 1.3 Goals (Business Outcomes)

| Goal | Success Criteria |
|------|------------|
| Meaning lives in one place | **Baseline**: metric semantics authored twice (dbt SQL plus Rust constants) and pre-computed. **Target**: each definition exists once as data; adding or changing a metric is a reviewed definition change, no generated-SQL machinery. **Timeframe**: end of Phase B. |
| Capability independent of data | **Baseline**: capability probed from observed rows; empty tenant reports none. **Target**: capability is a projection of definitions; a tenant with zero ingested rows has full authoring capability. **Timeframe**: end of Phase B. |
| One executor, no drift | **Baseline**: dbt-emitted rows and a separate query builder can diverge. **Target**: a single compiler serves every definition; cutover holds e2e parity (same seeds, same requests, same expectations). **Timeframe**: Phase B cutover. |
| Every read is scoped by construction | **Baseline**: tenancy is injected; org-scope and cohort scope are enforced only at the request boundary. **Target**: tenant, org-scope entity visibility, and cohort isolation are all injected server-side at the single compiler choke point; the client cannot widen them. **Timeframe**: end of Phase B. |
| Self-serve authoring without deploys | **Baseline**: adding a metric or chart is a code deploy. **Target**: an authorized user creates a measure, composes a metric, and places it on a dashboard with zero deploys and full audit history. **Timeframe**: end of Phase B. |

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Dataset | A queryable relation with guaranteed semantics — deduplicated, tenant-scoped, stable column names/types. Carries ingestion correctness (mutability, late data) so nothing above it re-solves those problems. Product-owned, with the one gated custom-dataset exception. |
| Custom dataset | A dataset-author-role registration of a SQL `SELECT` over dedup-safe catalog views. The single gated SQL layer; default off per tenant. |
| Field catalog | The typed, role-annotated schema generated from dataset schemas: the editor palette, the compiler's validation universe, and the discovery vocabulary. |
| Measure | A declarative aggregation of one dataset — the lowest editable layer and the atom every metric is built from. |
| Metric | A composition of measures into a served value: computation, optional post-aggregation shaping, and display identity (direction, format, naming). |
| Chart / dashboard | Pure presentation over a metric — view, dimension, layout, thresholds, targets. No query semantics. |
| Compiler | The single executor: definition plus request in, warehouse SQL out. Owns catalog resolution, rendering, read discipline, the time model, and all injected scopes. |
| Definition store | Application-owned storage of definitions with versioning, referential integrity, ownership/tenancy, and audit. |
| Materialization cache | A compiler-managed, version-keyed cache of aggregated work (not answers), refreshed on a schedule, never on read. |
| Injected scope | A server-side predicate the compiler adds to every query from request context — tenancy, org-scope entity visibility, cohort isolation — that the client cannot widen. |
| Org scope | The set of individuals within a viewer's org-chart reach (self plus related subtree), resolved from the identity service; the boundary for entity visibility and cohort composition. |
| Reporting timezone | The single declared per-tenant time zone in which the compiler buckets time; not a per-request parameter. |

## 2. Actors

### 2.1 Human Actors

#### Analyst / Viewer

**ID**: `cpt-semantic-layer-actor-analyst`

**Role**: Reads served metric values through charts and dashboards, requests by metric key with a scope, range, grain, dimensions, and view. Owns the correctness judgment on what a value means for their decision.
**Needs**: To read values that are correct, self-describing, and confined to their own scope, without ever seeing people or cohorts outside it.

#### Admin (Definition Author)

**ID**: `cpt-semantic-layer-actor-admin`

**Role**: Authors and edits measures, metrics, charts, and dashboards through structured editors over the field catalog. Product definitions are reviewed code; tenant definitions are this actor's runtime creations.
**Needs**: To compose and change analytical definitions at runtime, validated before they exist, with full audit history and no deploy.

#### Dataset Author

**ID**: `cpt-semantic-layer-actor-dataset-author`

**Role**: Holder of a distinct, per-tenant, default-off role who registers a SQL `SELECT` as a custom dataset — the sanctioned home for cross-dataset joins, sequence logic, and correlated comparisons the measure layer cannot express.
**Needs**: Full SQL expressiveness over the governed catalog read surface, with the platform guaranteeing tenancy, isolation, and resource safety around it.

### 2.2 System Actors

#### Compiler Service

**ID**: `cpt-semantic-layer-actor-compiler-svc`

**Role**: The analytics service (Rust) that owns the definition store, the single compiler, the field-catalog artifact, the materialization cache, and the discovery and definition APIs. It compiles every definition and injects every scope.

#### ClickHouse

**ID**: `cpt-semantic-layer-actor-clickhouse`

**Role**: External warehouse holding datasets (silver/gold relations and custom-dataset views) and the compiler-managed materialization cache. Executes the compiled SQL; enforces read discipline through the dataset views.

#### Identity Service

**ID**: `cpt-semantic-layer-actor-identity-svc`

**Role**: External authority for the org chart and entity visibility. Resolves the set of persons within a viewer's org scope, which the compiler injects as the entity-visibility scope. Authoritative over cohort composition.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **Single reporting timezone per tenant**: time bucketing happens in one declared per-tenant zone; there is no per-request time-zone override, so a request never changes meaning across executors or cache tiers.
- **Warehouse convergence is asynchronous**: deploys ship code and definitions only; the warehouse converges on its own schedule, so definitions referencing not-yet-present relations enter an explicit `unavailable` state rather than failing a deploy.
- **Custom SQL reads dedup-safe catalog views only**: raw source payloads, physical tables, and pipeline intermediates are not referenceable; read discipline is unbreakable by construction.

## 4. Scope

### 4.1 In Scope

- Definitions (datasets, measures, metrics, charts, dashboards) authored and stored as data in one canonical format.
- A single compiler that turns any definition plus request into warehouse SQL at query-time grain, with tenant-timezone bucketing.
- A gated custom-dataset layer: role-gated, default off, wrapped with tenancy and resource guardrails, reading dedup-safe catalog views only.
- Capability derived from definitions and configuration, never from stored derived rows.
- Server-injected scopes on every compiled query: tenant isolation, org-scope entity visibility, and cohort isolation.
- A materialization cache of aggregated work, version-keyed, refreshed on schedule; a discovery API; runtime editing in dependency order (charts/dashboards → metrics → measures → custom datasets).

### 4.2 Out of Scope

- Physically re-hosting or re-ingesting base facts — the dataset layer reuses existing bronze→silver ingestion and class contracts.
- A general-purpose BI platform beyond the editability boundary — moving that boundary is an explicit design decision, not incremental widening.
- A per-request time-zone parameter or client-composed queries — semantics are server-owned.
- **Safety-critical / life-critical behavior**: Not applicable — internal analytics platform, no physical actuation.
- **Accessibility, i18n, offline operation**: Not applicable — internal analytics platform with an authenticated, online operator audience for Phase B.
- **Payments, PCI, HIPAA**: Not applicable — internal analytics platform, no payment or regulated-health data handled.

## 5. Functional Requirements

> **Testing strategy**: All requirements verified via automated tests (unit, integration, e2e) targeting 90%+ code coverage unless otherwise specified. The declarative e2e metric suite is the cutover parity invariant.

### 5.1 Definitions and Compiler (p1)

#### Definitions as Data

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-definitions-as-data`

The system **MUST** represent every analytical definition — dataset, measure, metric, chart, dashboard — as a validated row in a definition store in one canonical format, never as code. Product-shipped and user-created definitions **MUST** be the same kind of object, differing only in ownership. An invalid definition **MUST NOT** be writable.

**Rationale**: Anything a user may eventually edit must be data so it can be authored, versioned, audited, and validated uniformly.

**Actors**: `cpt-semantic-layer-actor-admin`, `cpt-semantic-layer-actor-compiler-svc`

The authoring-as-data first step shipped: the metric registry moved from Rust constants to a validated YAML registry (#1974). That registry still uses the transitional observation-relation shape; the target rewrites it to the dataset/measure/metric domain model. This requirement stays open until the domain-model store is the single runtime source of truth for every consumer.

#### One Compiler over Datasets

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-one-compiler`

The system **MUST** serve every definition through a single compiler that takes a definition plus a request (entity scope, date range, grain, dimensions, filters) and produces warehouse SQL. Product and user definitions **MUST** compile through the same path. A second executor **MUST** exist only as a bounded transitional state during cutover.

**Rationale**: Two interpreters of one definition format diverge on nulls, time zones, and deduplication; one executor removes that drift class structurally.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-clickhouse`

#### Gated Custom-Dataset SQL Layer

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-gated-custom-dataset`

The system **MUST** allow a holder of the dataset-author role (per-tenant, default off) to register a SQL `SELECT` as a custom dataset. The statement **MUST** read only dedup-safe catalog views (never raw tables, payloads, or pipeline intermediates); its references **MUST** be checked against the catalog and its result columns annotated with catalog roles. Execution **MUST** be wrapped so the tenancy predicate is applied from outside the statement and resource guardrails bound it. Deletion **MUST** be blocked while measures reference it.

**Rationale**: Expressiveness the structured layers cannot reach (joins, windows, sequence logic) needs exactly one gated home with dataset-sized blast radius, safe by construction.

**Actors**: `cpt-semantic-layer-actor-dataset-author`, `cpt-semantic-layer-actor-compiler-svc`

#### Capability from Definitions

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-capability-from-definitions`

The system **MUST** derive what can be asked (metric and measure catalog, dimension keys) from shipped code and stored definitions, never from stored derived rows. A tenant with zero ingested rows **MUST** report full authoring capability. Data values **MUST** determine only which concrete filter values exist, served through a dedicated distinct-values path with an explicit unavailable state.

**Rationale**: Capability inferred from data makes empty tenants dead, lets partial ingestion mutate the contract, and lets ingestion defects silently remove features.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-admin`

#### Server-Owned Semantics

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-server-owned-semantics`

The system **MUST** own all query semantics — filters, formulas, windows, formatting identity — on the server. Clients **MUST** request by key and render what they are given, holding no metric vocabulary, dimension lists, or validation semantics. Query responses **MUST** be self-describing and carry provenance (definition version, cache-or-computed, data-available-from, availability state).

**Rationale**: Meaning fragments and drifts if any of it lives in a client; server-owned meaning makes it structural rather than conventional.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-analyst`

### 5.2 Time and Scope (p1)

#### Query-Time Grain and Reporting Timezone

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-query-time-grain`

The system **MUST** treat grain (day, week, month, quarter) as a query-time parameter, never baked into stored data, and **MUST** bucket time in a single declared per-tenant reporting time zone with no per-request override. Windowed and cumulative semantics **MUST** be compiler features parameterized at request time, not encoded ad hoc into definitions.

**Rationale**: Grain baked into materializations and per-request time zones both break server-owned semantics and re-aggregation.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-analyst`

#### Tenant Isolation

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-tenant-isolation`

The system **MUST** inject a tenancy predicate on every query the compiler emits, sourced from request context and not from any definition or client input. No definition **MUST** be able to widen or remove it.

**Rationale**: Multi-tenant isolation must be a property of the single choke point, not of each definition.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-clickhouse`

#### Org-Scope Entity Visibility

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-org-scope-visibility`

The system **MUST** inject an entity-visibility scope on every compiled query so a viewer reads per-entity values only for individuals within their org-chart scope (self plus related subtree). People outside the scope **MUST NOT** be returned and their existence **MUST NOT** be disclosed. The visible set **MUST** be resolved from the org chart owned by the identity service, injected server-side beside the tenancy predicate, and **MUST** fail closed if the authorization source is unavailable.

**Rationale**: "No people outside your org scope" is a required security guarantee that must hold uniformly for every read path, not per endpoint.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-identity-svc`

This scope is partially enforced today at the request boundary: analytics resolves the caller's org-visible set via `person_visibility` → identity `/v1/visible-persons` and refuses out-of-scope ids (leaking only a count). Under compiler-first it moves into the compiler's shared `WHERE` as an injected `entity ∈ visible_set` filter; definitions stay scope-agnostic. This requirement stays open until every compiled read path carries the injected scope.

#### Cohort Scope Isolation

- [ ] `p1` - **ID**: `cpt-semantic-layer-fr-scope-isolation`

The system **MUST** prevent per-entity reads for unrelated teams or cohorts. A person's peer cohort **MUST** be drawn from within their org-chart scope; tags and attributes **MUST** only refine a cohort inside that boundary and **MUST NOT** pull a person across it. Cross-cohort comparison **MUST** be aggregates-only: peer views return distributions with no member ids and **MUST** be suppressed below a minimum distinct-member floor so small groups cannot disclose individuals.

**Rationale**: "No other teams/cohorts" — a shared tag must never route around org-scope isolation, and small cohorts must not re-identify members.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-identity-svc`

### 5.3 Serving and Authoring (p2)

#### Discovery API

- [ ] `p2` - **ID**: `cpt-semantic-layer-fr-discovery-api`

The system **MUST** tell clients what exists rather than let them hardcode or probe: the catalog of metrics and measures visible to the tenant (product ∪ tenant) with computation, format, direction, and allowed dimensions; for editors, dataset field catalogs with roles and types and a read-only display body per dataset; and on demand, distinct dimension values with an explicit unavailable state. Requests **MUST** be validated against the same catalog discovery serves.

**Rationale**: The frontend must be a renderer of server-owned definitions; discovery is the single vocabulary that keeps requests and capability from disagreeing.

**Actors**: `cpt-semantic-layer-actor-analyst`, `cpt-semantic-layer-actor-admin`

#### Materialization Cache

- [ ] `p2` - **ID**: `cpt-semantic-layer-fr-materialization-cache`

The system **MUST** treat materialized results as a compiler-managed cache of aggregated work (not request→response answers), keyed by definition version, invisible to the definition contract. Refresh **MUST** be scheduled, never triggered by a read, and every degraded state (stale version, uncovered range, disabled policy, unavailable definition) **MUST** fall back to live compute rather than serve a wrong or superseded value.

**Rationale**: Caching is a latency policy, not semantics; the read path must stay read-only and never serve values computed under a superseded definition.

**Actors**: `cpt-semantic-layer-actor-compiler-svc`, `cpt-semantic-layer-actor-clickhouse`

#### Runtime Editing

- [ ] `p2` - **ID**: `cpt-semantic-layer-fr-runtime-editing`

The system **MUST** let authorized users create and edit definitions at runtime through role-gated, tenant-namespaced editors, opening layers in dependency order (charts/dashboards, then metrics, then measures, then gated custom datasets). Each write **MUST** run the same validators as seed reconciliation, bump the definition version on semantic change, append an audit revision, and be blocked from deleting a referenced definition.

**Rationale**: Runtime authoring without audit or shared validators is not acceptable in a multi-tenant product; the order keeps each layer opening only over a validated one below it.

**Actors**: `cpt-semantic-layer-actor-admin`, `cpt-semantic-layer-actor-dataset-author`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Source Read Discipline

- [ ] `p1` - **ID**: `cpt-semantic-layer-nfr-source-read-discipline`

Every read the compiler emits **MUST** apply the dataset's declared read discipline (deduplication, mutable-read handling) inherited from dataset metadata, and custom SQL **MUST** be unable to query around it because it reads only dedup-safe catalog views.

**Threshold**: 0 reads that bypass dataset deduplication in adversarial testing; 0 references to raw tables or pipeline intermediates admitted by the custom-dataset validator.

**Rationale**: Correctness of every value above the dataset line depends on read discipline being unbreakable by construction.

#### Tenant Isolation

- [ ] `p1` - **ID**: `cpt-semantic-layer-nfr-tenant-isolation`

Compiled reads for tenant A **MUST NOT** return rows for tenant B, regardless of definition content or client input.

**Threshold**: 0 cross-tenant rows returned in isolation testing across every compiled read path.

**Rationale**: Multi-tenant SaaS isolation guarantee.

#### Entity Visibility

- [ ] `p1` - **ID**: `cpt-semantic-layer-nfr-entity-visibility`

A viewer's compiled reads **MUST NOT** return per-entity values for individuals outside their org scope, and out-of-scope existence **MUST NOT** be disclosed. The scope source being unavailable **MUST** fail closed.

**Threshold**: 0 out-of-scope persons returned or disclosed in scope testing; unavailable authorization source yields no rows, never a widened read.

**Rationale**: "No people outside your org scope" is a hard security boundary.

#### Cohort Scope Isolation

- [ ] `p1` - **ID**: `cpt-semantic-layer-nfr-cohort-scope-isolation`

Peer and cohort reads **MUST NOT** cross an org boundary for any cohort key, and aggregate peer views **MUST NOT** disclose members below the distinct-member floor.

**Threshold**: 0 cross-boundary cohort members for any cohort key (not only `org_unit`); 0 peer views served below the minimum distinct-member floor.

**Rationale**: A shared tag must not route around scope isolation, and small cohorts must not re-identify individuals.

#### Executor Consistency

- [ ] `p1` - **ID**: `cpt-semantic-layer-nfr-executor-consistency`

Cutover to the single compiler **MUST** preserve behavior: the declarative e2e suite **MUST** pass with the same bronze seeds, the same requests, and the same expectations against the new executor.

**Threshold**: 100% of the existing e2e metric expectations green against the compiler path; any divergence class (dedup, timezone, null propagation) resolved deliberately before flip.

**Rationale**: The e2e suite is the parity invariant that makes the store rewrite and executor swap safe.

#### Query Guardrails

- [ ] `p2` - **ID**: `cpt-semantic-layer-nfr-query-guardrails`

Every query the compiler emits — product, tenant, or custom-dataset — **MUST** run under resource guardrails (row caps, timeout classes, memory bounds) so a pathological definition degrades to an error, not an incident.

**Threshold**: 100% of compiled queries bounded by a guardrail; a definition exceeding a bound fails loudly rather than running unbounded.

**Rationale**: Runtime-created definitions can be pathological; guardrails make that a performance event, not a correctness or availability event.

### 6.2 NFR Exclusions

- **Per-request query latency SLA**: Not specified for Phase B — materialization is pulled forward per measure only where shadow-phase latency evidence warrants it; no committed per-request latency target.
- **High availability / clustering**: Managed at ClickHouse infrastructure level, not by this module.
- **Encryption at rest**: Handled by ClickHouse infrastructure configuration.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Query API

- [ ] `p1` - **ID**: `cpt-semantic-layer-interface-query-api`

**Type**: REST API (HTTP/JSON)

**Stability**: stable

**Description**: Request a served value by metric key with entity scope, range, grain, dimensions, filters, and view. Responses are self-describing and carry provenance. This is the contract Phase B holds stable across the executor cutover; detailed endpoint contracts live in DESIGN.

**Breaking Change Policy**: Stable — the response shape is a presentation contract, orthogonal to how values are computed; changes are additive.

#### Discovery API

- [ ] `p2` - **ID**: `cpt-semantic-layer-interface-discovery-api`

**Type**: REST API (HTTP/JSON)

**Stability**: unstable

**Description**: The catalog of visible metrics and measures with capability, dataset field catalogs for editors, and paginated distinct dimension values with an explicit unavailable state. Everything a client renders derives from here.

**Breaking Change Policy**: Unstable in Phase B; grows additively.

#### Definitions API

- [ ] `p2` - **ID**: `cpt-semantic-layer-interface-definitions-api`

**Type**: REST API (HTTP/JSON)

**Stability**: unstable

**Description**: Role-gated CRUD per definition layer, running the same validators as seed reconciliation, with validate-only dry runs, fork, and cache-policy changes. This surface is also the agent/MCP authoring surface — machine authors get the same endpoints and validation.

**Breaking Change Policy**: Unstable in Phase B; hardens as runtime editing opens per layer.

### 7.2 External Integration Contracts

#### Org-Chart Visibility Consumption

- [ ] `p1` - **ID**: `cpt-semantic-layer-contract-visibility-consumption`

**Direction**: required from external (the compiler resolves each viewer's org-visible person set from the identity service)

**Protocol/Format**: Token-forwarded lookup of the caller's visible-persons set, resolved from the identity-owned org chart; the compiler injects the result as an entity-visibility scope.

**Compatibility**: Fail-closed if unavailable; the visible-set contract is authoritative over cohort composition and never widened by definitions.

#### Warehouse Read Contract

- [ ] `p2` - **ID**: `cpt-semantic-layer-contract-warehouse-read`

**Direction**: required from external (the compiler reads datasets and the cache from ClickHouse)

**Protocol/Format**: Compiled `SELECT` over dataset relations and dedup-safe catalog views; the materialization cache is a compiler-managed relation.

**Compatibility**: Dataset contracts evolve additively; a relation lagging its declared contract puts affected definitions into an `unavailable` state rather than failing.

## 8. Use Cases

### Author a Measure and a Metric

- [ ] `p1` - **ID**: `cpt-semantic-layer-usecase-author-measure-metric`

**Actor**: `cpt-semantic-layer-actor-admin`

**Preconditions**:
- The field catalog for the target dataset is available.
- The admin holds the measure/metric editing role for the tenant.

**Main Flow**:
1. Admin opens the measure editor over a dataset's field catalog and composes an aggregation (filter tree, aggregation kind, event time, entity, dimensions).
2. System validates the definition against the catalog and expression allowlist and writes it, bumping the version and appending a revision.
3. Admin composes a metric over one or more measures (computation, transform, display identity).
4. System validates and stores the metric; capability appears immediately in discovery.

**Postconditions**:
- The measure and metric exist as validated store rows; the compiler can serve them at any grain with zero materialization.

**Alternative Flows**:
- **Validation fails**: System rejects the write with a precise field-level error; nothing is stored.

### Register a Custom Dataset

- [ ] `p2` - **ID**: `cpt-semantic-layer-usecase-register-custom-dataset`

**Actor**: `cpt-semantic-layer-actor-dataset-author`

**Preconditions**:
- The dataset-author role is enabled for the tenant.
- The referenced catalog datasets exist.

**Main Flow**:
1. Author submits a SQL `SELECT` over dedup-safe catalog views and annotates result columns with catalog roles.
2. System parses the statement, checks references against the catalog, captures the result schema, and rejects any reference to raw tables or intermediates.
3. System registers the annotated schema as an ordinary catalog entry; measures may compose on top.

**Postconditions**:
- The custom dataset is a versioned catalog entry, query-time by default, promotable to cache; lineage marks everything built on it.

**Alternative Flows**:
- **Reference or role violation**: System rejects registration; nothing enters the catalog.

### Promote a Definition to a Dashboard

- [ ] `p2` - **ID**: `cpt-semantic-layer-usecase-promote-to-dashboard`

**Actor**: `cpt-semantic-layer-actor-admin`

**Preconditions**:
- A validated metric exists and is discovery-visible.

**Main Flow**:
1. Admin creates a chart definition referencing the metric (view, dimension, thresholds).
2. Admin places the chart on a dashboard definition (layout).
3. System validates references, stores the definitions, and blocks deletion of the referenced metric.

**Postconditions**:
- The dashboard resolves with its charts and their metrics' current capability in one read; no deploy occurred.

**Alternative Flows**:
- **Referenced metric missing/unavailable**: System surfaces the availability state in discovery; the chart renders its error state.

## 9. Acceptance Criteria

- [ ] Every product metric and measure exists as a validated definition row in the canonical domain-model format, authored once.
- [ ] A single compiler serves every definition; the e2e suite passes with the same seeds, requests, and expectations against the compiler path.
- [ ] A tenant with zero ingested rows reports full product authoring capability.
- [ ] Every compiled query carries an injected tenancy predicate the client cannot widen; cross-tenant reads return no rows.
- [ ] Every compiled per-entity read is confined to the viewer's org scope; out-of-scope persons are never returned or disclosed, and an unavailable authorization source fails closed.
- [ ] No cohort read crosses an org boundary for any cohort key, and peer views below the distinct-member floor are suppressed.
- [ ] Grain is a query-time parameter and time buckets in the tenant reporting time zone; no grain is baked into a contract-bearing materialization.
- [ ] An authorized user creates a measure, composes a metric, and places it on a dashboard with zero deploys and full audit history.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| ClickHouse | Stores datasets and the materialization cache; executes compiled SQL | `p1` |
| Analytics service (Rust) | Hosts the definition store, compiler, discovery and definition APIs | `p1` |
| Identity service | Resolves org-chart visibility and is authoritative over cohort composition | `p1` |
| dbt / ingestion (class contracts) | Produces the datasets the measure layer builds on; carries dedup/mutability correctness | `p1` |
| Declarative e2e metric suite | The cutover parity invariant (same seeds, requests, expectations) | `p1` |
| Presentation-layer Phase A boundary | The read-only, tenant-scoped boundary this phase builds semantics on | `p2` |

## 11. Assumptions

- Existing bronze→silver ingestion and class contracts are reused as the dataset layer; base facts are not re-ingested.
- Product definitions are authored and reviewed as repo files and reconciled into the store at startup; the store is the single runtime source of truth.
- The reporting time zone is a per-tenant profile setting (default UTC) with no per-request override.
- Materialization is the exception, not the default: query-time compute is the default, cache pulled forward only where latency evidence warrants.
- The identity service is the authority for org-chart visibility and cohort composition; analytics never invents scope.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Executor drift during cutover | Compiler and legacy path disagree on nulls/timezone/dedup | Shadow-compare per family against the e2e invariant; resolve each divergence class before flip; delete the flag at deletion phase |
| Expression allowlist mis-calibration | Too narrow blocks valid measures; too wide leaks unsafe SQL | Start narrow; widen only by explicit reviewed additions the parser makes visible |
| Injected scope missing on a new read path | Cross-tenant, out-of-scope, or cross-cohort disclosure | Inject all scopes at the single compiler choke point; adversarial isolation tests over every compiled path; fail closed on unavailable authorization |
| Cohort key spanning the org boundary | A tag-based cohort discloses across scope | Enforce org-gating where cohort membership is produced and re-assert it as the compiler's injected scope for every cohort key |
| Serving a superseded definition from cache | Stale semantics presented as truth | Version-key every cached row; fall back to live compute on version mismatch, uncovered range, or unavailable definition |
| Runtime-created pathological definition | Unbounded query load | Bound every compiled query with row caps, timeout classes, and memory settings |
