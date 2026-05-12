# PRD — Connector Framework

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
  - [3.2 Expected Scale](#32-expected-scale)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Core Framework](#51-core-framework)
  - [5.2 Bronze Layer](#52-bronze-layer)
  - [5.3 Silver Layer / Unification](#53-silver-layer--unification)
  - [5.4 Security & Access Control](#54-security--access-control)
  - [5.5 Data Classification & Privacy](#55-data-classification--privacy)
  - [5.6 Connector Authorship](#56-connector-authorship)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [8.1 Connector Development](#81-connector-development)
  - [8.2 Connector Operations](#82-connector-operations)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

The Connector Framework is the data ingestion subsystem of the Insight platform. It enables the platform to collect raw data from external source systems — version control, task tracking, HR directories, communication tools, AI development tools, CRM, and quality/testing — and deliver it through the Medallion Architecture (Bronze → Silver → Gold) for analytics consumption.

The framework separates two fundamentally different concerns: structural integration mechanics (authentication, pagination, rate limiting, error recovery) that are identical across all sources, and semantic mapping (cross-source unification, enum normalization, identity resolution rules) that requires human authorship per domain.

### 1.2 Background / Problem Statement

Organizations typically use 5–15+ SaaS tools across development, HR, communication, and project management workflows. Extracting meaningful cross-tool analytics requires solving several compounding problems:

1. **Ad-hoc integrations** — without a framework, each data source integration is built from scratch, duplicating authentication, pagination, and error handling logic. Quality and reliability vary per integration.
2. **Schema fragility** — source APIs evolve independently. Without versioned contracts, schema changes silently break downstream analytics pipelines, discovered only when dashboards show incorrect data.
3. **Semantic inconsistency** — different sources model similar concepts differently (e.g., "task status" in YouTrack vs Jira). Without explicit unification rules, cross-source analytics produce misleading comparisons.
4. **Scaling bottleneck** — with 2,000+ potential customers each using different tooling stacks, the platform team cannot build every connector. External authors (community contributors, customers) need a supported path to build connectors without deep platform knowledge.
5. **Mixed concerns** — when structural mechanics and semantic decisions are interleaved in connector code, every connector becomes a maintenance liability. Changes to the framework require touching every connector; new connectors re-invent solved problems.

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- New connector Bronze-layer development reduced from weeks to days by eliminating re-implementation of common concerns (Baseline: ~2 weeks for first connectors; Target: <3 days for Bronze layer; Timeframe: v1.0)
- External connector authorship enabled through a published SDK — community and self-service developers can build connectors without platform team involvement (Baseline: 0 external connectors; Target: SDK published with documentation and example connector; Timeframe: v1.0)
- Zero silent schema breakage — all connector schema changes validated before deployment; incompatible changes detected and blocked (Baseline: no validation; Target: 100% of schema changes validated; Timeframe: v1.0)

**Capabilities**:

- [ ] Provide a reusable framework handling all common integration concerns for any external data source
- [ ] Store raw collected data preserving source-native schema and identifiers (Bronze layer)
- [ ] Unify data from multiple sources of the same domain into canonical schemas (Silver layer)
- [ ] Enable three tiers of connector authorship: first-party, community, and self-service
- [ ] Propagate semantic metadata from connector definitions to downstream analytics systems

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Connector | A configured integration with a specific external data source instance, responsible for extracting data and delivering it to the Bronze layer |
| Source Instance | A single deployment of an external system, e.g., a company's Jira Cloud, a team's GitHub organization. Identified by a stable `insight_source_id` |
| Bronze Layer | The raw data layer in the Medallion Architecture. Stores collected data preserving source-native schema and identifiers. One set of tables per source |
| Silver Layer | The unified data layer. Cross-source normalized data with canonical `class_{domain}` schemas. Silver tables retain source-native user identifiers; identity resolution is not applied at this layer but Silver records can be joined with the Identity Manager's `person_id` mapping at query time or in Gold |
| Gold Layer | The derived metrics layer. Pre-aggregated analytics consuming exclusively from Silver tables. Domain-specific names, no raw events |
| Medallion Architecture | The Bronze → Silver → Gold data pipeline pattern used by the platform |
| Connector Author | A person developing a new connector — may be a platform engineer (first-party), open-source contributor (community), or customer (self-service) |
| Unifier | A domain-level definition mapping source-specific fields to unified Silver schema fields, including semantic annotations |
| Collection Run | One execution of a connector's data extraction cycle. Recorded for auditing with timing, record counts, and error details |
| Semantic Metadata | Per-field annotations (display name, description, aggregation rule, applicable teams) attached to unified schema fields, propagated to downstream analytics systems |
| PII (Personal Identifiable Information) | Data attributable to an individual — names, emails, employee IDs, job titles. Collected primarily by HR and communication connectors. Subject to GDPR requirements |
| Data Sensitivity Level | Classification of a collected field: Public metadata, Internal metadata, Personal data (PII), or Sensitive content. Declared in the connector specification |
| Backfill | Initial historical data loading when a new connector instance is activated. Covers a configurable lookback window (e.g., 30 days to full history) |
| Lookback Window | The depth of historical data to load during backfill. Configured per connector instance |
| Silver Schema | The canonical `class_{domain}` table definition that all connectors of a domain map to. Versioned independently from connector specifications |
| Resource Quota | Configurable limit on CPU, memory, and temporary storage for a single collection run |
| Quarantine (Silver) | A holding area for Bronze records that fail Silver unification, preserving the original data and the error reason for re-processing |
| Cold Storage | Cost-optimized archival tier for Bronze data that has exceeded the active retention period |
| Data Lineage | Traceable reference from a Silver record back to the originating Bronze record(s), preserving source type, source instance, and Bronze primary key |

## 2. Actors

> **Note**: Stakeholder needs are managed at project/task level by steering committee. Document **actors** (users, systems) that interact with this module.

### 2.1 Human Actors

#### Platform Engineer

**ID**: `cpt-insightspec-actor-cn-platform-engineer`

**Role**: Develops and maintains first-party connectors and the connector framework itself. Defines domain unifier schemas. Reviews and approves community-contributed connectors.
**Needs**: Minimal boilerplate when building new connectors; clear separation between framework concerns and source-specific logic; ability to evolve the framework without modifying every connector.

#### Connector Author

**ID**: `cpt-insightspec-actor-cn-connector-author`

**Role**: External developer (community contributor or customer) building a new connector using the published SDK. Does not have direct access to the platform codebase.
**Needs**: Well-documented SDK with clear contract; example connectors as reference; ability to test a connector locally before submission; feedback on specification validity.

#### Workspace Admin

**ID**: `cpt-insightspec-actor-cn-workspace-admin`

**Role**: Configures connector instances for their organization — selects data sources, provides credentials, monitors collection health, responds to collection failures.
**Needs**: Visibility into collection status and errors; ability to enable/disable connectors; credential management without platform engineering involvement.

#### Data Analyst

**ID**: `cpt-insightspec-actor-cn-data-analyst`

**Role**: Consumes analytics derived from collected data at the Gold layer. Does not interact with connectors directly but depends on reliable, timely, and semantically correct data.
**Needs**: Consistent data freshness across sources; trustworthy cross-source comparisons; clear field descriptions and metadata in analytics tools.

### 2.2 System Actors

#### Source API

**ID**: `cpt-insightspec-actor-cn-source-api`

**Role**: External SaaS system providing data via HTTP/REST/GraphQL APIs. Each source has its own authentication scheme, pagination model, rate limits, and data schema.

#### Identity Manager

**ID**: `cpt-insightspec-actor-cn-identity-manager`

**Role**: Internal Insight platform service that maintains a canonical `person_id` registry by mapping source-native user identifiers (emails, logins, employee IDs) to a single person identity. Silver tables are NOT enriched by the Identity Manager — they retain source-native user IDs. The `person_id` mapping is available for joining at query time or in Gold layer aggregations. Maintained as a separate domain (see `docs/domain/identity-resolution/`).

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Connectors MUST operate in network-connected environments with access to source system APIs
- Connectors MUST support running in isolated execution contexts — no shared state between connector instances
- The framework MUST support multiple instances of the same source type within a single workspace (e.g., two separate GitHub organizations)

### 3.2 Expected Scale

| Dimension | Current (v1.0 launch) | Projected (12 months) |
|-----------|----------------------|----------------------|
| Workspaces | 10–50 | 200–500 |
| Connector types (source adapters) | 15–20 | 30–50 |
| Connector instances per workspace | 3–10 | 5–20 |
| Total active connector instances | 50–250 | 1,000–5,000 |
| Records per connector run (typical) | 100–50,000 | 100–100,000 |
| Records per connector run (peak, e.g., GitHub commits) | Up to 500,000 | Up to 1,000,000 |
| Collection runs per day (total across all workspaces) | 200–1,000 | 5,000–25,000 |
| Bronze storage growth per month | 5–20 GB | 50–200 GB |

These projections inform capacity planning in the DESIGN phase. The framework MUST be designed to handle the projected scale without architectural changes.

## 4. Scope

### 4.1 In Scope

- Connector framework providing common integration concerns (authentication, pagination, rate limiting, error recovery, run logging)
- Declarative connector specification contract
- Bronze layer data extraction and storage with source-native schema preservation
- Support for source-specific custom fields without core schema changes
- Collection run auditing and monitoring
- Connector SDK for external authorship (community and self-service tiers)
- Silver layer unification — cross-source normalization via domain schemas
- Semantic metadata definition and propagation
- Connector specification versioning and compatibility validation
- AI-assisted Silver layer mapping proposals (with mandatory human approval)

### 4.2 Out of Scope

- Connector orchestration and scheduling (separate domain: `docs/components/connectors-orchestrator/`)
- Identity Manager internals and person resolution logic (separate Insight platform domain: `docs/domain/identity-resolution/`)
- Gold layer metric definitions and aggregations (downstream consumer of Silver data)
- Dashboard and visualization layer
- Individual connector specifications (each connector has its own PRD/DESIGN)
- Data access permissions and workspace-level isolation enforcement (separate domain: `docs/architecture/permissions/`)
- Credential storage and secrets management infrastructure

## 5. Functional Requirements

> **Testing strategy**: All requirements verified via automated tests (unit, integration, e2e) targeting 90%+ code coverage unless otherwise specified. Document verification method only for non-test approaches (analysis, inspection, demonstration).

### 5.1 Core Framework

#### Common Integration Concerns

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-common-concerns`

The framework MUST handle authentication, pagination, rate limiting, and error recovery for all connectors. Connector authors MUST NOT need to implement these concerns. The framework MUST support multiple authentication schemes and pagination patterns declared in the connector specification.

**Rationale**: Eliminates duplicated effort across connectors; ensures consistent reliability and compliance with source API constraints.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`, `cpt-insightspec-actor-cn-connector-author`

#### Declarative Connector Specification

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-connector-spec`

Each connector MUST be described by a declarative specification defining: source identity, authentication method, available data entities, extraction capabilities, and rate limit parameters. The framework MUST use this specification to drive extraction behavior without source-specific code for structural concerns.

**Rationale**: Separates structural mechanics from source-specific logic; enables validation and code generation from a single source of truth.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`, `cpt-insightspec-actor-cn-connector-author`

#### Incremental Data Extraction

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-incremental-sync`

The system MUST support incremental data extraction — collecting only new or changed data since the last successful extraction run. The extraction position (cursor) MUST be stored externally to the connector process and MUST be updated only upon successful completion of a run.

**Rationale**: Full data reloads are prohibitively expensive for large source instances; incremental sync enables frequent, lightweight collection runs.

**Actors**: `cpt-insightspec-actor-cn-source-api`

#### Historical Data Backfilling

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-backfill`

The system MUST support initial historical data loading (backfill) when a new connector instance is activated. The connector specification MUST declare a configurable lookback window (e.g., 30 days, 1 year, full history). The system MUST implement a two-phase backfill strategy: (1) load recent data first (configurable "fast-start" window, default 30 days) so analytics become available immediately; (2) load remaining historical data in the background without blocking incremental collection of new data. Backfill runs MUST respect source API rate limits and MUST NOT starve concurrent incremental collection runs. At the platform level, incremental sync MUST always have priority over background backfill — when total throughput approaches storage write capacity or network bandwidth limits, backfill runs MUST yield to incremental runs. Backfill progress MUST be tracked and resumable — an interrupted backfill MUST continue from where it stopped, not restart.

**Rationale**: Every new customer onboarding requires loading months or years of historical data. Without a structured backfill strategy, initial loads consume days, exhaust API rate limits, and delay time-to-value. The two-phase approach ensures analytics are usable within hours, not days.

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`, `cpt-insightspec-actor-cn-source-api`

#### Idempotent Extraction

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-idempotent-extraction`

Re-running an extraction for the same data range MUST produce identical results in the Bronze layer. Failed runs MUST NOT advance the extraction position, ensuring the next run retries from the same point.

**Rationale**: Guarantees data consistency during retries and recovery scenarios; prevents data loss or duplication from transient failures.

**Actors**: `cpt-insightspec-actor-cn-source-api`

#### Error Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-error-isolation`

Failure of one connector MUST NOT affect data collection from other connectors. Each connector MUST run in an isolated execution context with no shared mutable state. Errors MUST be logged with structured details sufficient for diagnosis.

**Rationale**: With dozens of connectors running concurrently, a single source API outage must not cascade into platform-wide data collection failure.

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`

#### Collection Run Auditing

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-collection-audit`

Every extraction run MUST be recorded with: start time, completion time, status (running/completed/failed), record counts per entity type, API call count, and error details. This audit trail MUST be queryable for monitoring and diagnostics.

**Rationale**: Workspace admins need visibility into collection health; platform engineers need diagnostics for troubleshooting; data analysts need to understand data freshness.

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`, `cpt-insightspec-actor-cn-platform-engineer`

### 5.2 Bronze Layer

#### Raw Data Preservation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-raw-storage`

Raw data from each source MUST be stored preserving the source-native schema and identifiers. Each source MUST have its own set of Bronze tables. Source-specific structural quirks (nested objects, multi-valued fields) MUST be represented faithfully.

**Rationale**: Bronze is the system of record for collected data; downstream transformations must always be reproducible from Bronze.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`

#### Standard Metadata on Every Record

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-standard-metadata`

Every collected record MUST carry: collection timestamp, data source identifier, and source instance identifier. These fields MUST be injected by the framework — connector authors MUST NOT populate them manually.

**Rationale**: Enables provenance tracking, multi-instance disambiguation, and freshness monitoring across all sources uniformly.

**Actors**: `cpt-insightspec-actor-cn-data-analyst`

#### Custom Fields Support

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-custom-fields`

The system MUST support source-specific custom fields (e.g., custom Jira fields, BambooHR custom attributes) without requiring changes to the core Bronze table schema. Custom fields MUST be stored in a queryable format alongside the core entity data.

**Rationale**: Every customer configures source systems differently; custom fields contain business-critical data that must be collected without per-customer schema modifications.

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`, `cpt-insightspec-actor-cn-data-analyst`

#### Bronze Data Lifecycle

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-bronze-lifecycle`

The system MUST support configurable retention policies for Bronze data per workspace. Once Bronze data has been successfully unified into Silver, and the retention period has elapsed, the system MUST support automatic archival to cold storage or deletion. The default retention period MUST be defined at the platform level (e.g., 2 years) and MAY be overridden per workspace within platform-defined bounds. Workspace admins MUST be notified before data is archived or deleted. Data under active PII retention holds (see `cpt-insightspec-fr-cn-pii-retention`) MUST NOT be archived or deleted until the hold is released.

**Rationale**: At a projected 50–200 GB/month Bronze growth, unbounded raw data retention becomes a significant cost driver. Without a lifecycle policy, storage costs grow linearly with no upper bound, and most Bronze data older than 1–2 years is never queried directly (Silver/Gold serve analytics).

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`

### 5.3 Silver Layer / Unification

#### Cross-Source Unification

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-cross-source-unification`

Data from multiple sources of the same domain (e.g., GitHub + Bitbucket + GitLab for version control) MUST be unified into a single canonical schema per domain (`class_{domain}` Silver tables). Unification MUST include field mapping, enum normalization, and unit conversions.

**Rationale**: Cross-source analytics (e.g., comparing development velocity across teams using different tools) require a single unified schema; without it, every downstream query must handle source-specific variations.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`, `cpt-insightspec-actor-cn-data-analyst`

#### Bronze-to-Silver Lineage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-data-lineage`

Every record in a Silver `class_{domain}` table MUST retain a traceable reference to the originating Bronze record (source type, source instance, and Bronze primary key). This lineage link MUST allow navigating from any unified Silver record back to the exact raw Bronze row it was derived from. The lineage MUST be preserved through the Silver unification process. When a Silver record is derived from multiple Bronze records (e.g., merge or deduplication), all contributing Bronze references MUST be retained.

**Rationale**: Without explicit lineage, debugging data quality issues in Gold/Silver requires guesswork about which source and which raw record produced the anomaly. Lineage is also a prerequisite for the quarantine re-processing flow and for auditing the unification logic when Silver mappings are disputed.

**Actors**: `cpt-insightspec-actor-cn-data-analyst`, `cpt-insightspec-actor-cn-platform-engineer`

#### Unification Failure Quarantine

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-silver-quarantine`

Records that successfully reach Bronze but fail Silver unification (e.g., unexpected enum values, missing required fields, data type mismatches from source API changes) MUST NOT be silently dropped. The system MUST route such records to a quarantine area, preserving the original Bronze data and the unification error reason. Quarantined record counts MUST be visible to workspace admins and data analysts as a data quality metric (percentage of records that "fell out" of analytics). The system MUST support re-processing quarantined records after a unifier mapping is corrected.

**Rationale**: Silent data loss at the Silver boundary is invisible to analysts — dashboards show correct-looking but incomplete data. A quarantine mechanism makes data quality gaps explicit and recoverable.

**Actors**: `cpt-insightspec-actor-cn-data-analyst`, `cpt-insightspec-actor-cn-platform-engineer`

#### Semantic Metadata Propagation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-semantic-metadata`

Each unified Silver field MUST carry semantic annotations: display name, description, aggregation rule, and applicable team/domain context. These annotations MUST propagate automatically to downstream analytics systems (Data Catalog, Semantic Dictionary) without manual configuration.

**Rationale**: Data analysts need to understand what each field means and how to aggregate it; manual metadata maintenance does not scale across hundreds of fields.

**Actors**: `cpt-insightspec-actor-cn-data-analyst`

#### AI-Assisted Mapping with Human Approval

- [ ] `p3` - **ID**: `cpt-insightspec-fr-cn-ai-assisted-mapping`

The system SHOULD provide AI-generated proposals for mapping new source fields to unified Silver schemas — including field-to-column candidates, enum clustering, and unit detection. All AI proposals MUST require explicit human review and approval before being applied. No semantic mapping decision may be automated without human sign-off.

**Rationale**: Eliminates the "blank page" problem when onboarding a new source, while ensuring semantic correctness remains a human responsibility — AI proposals have been shown to miss domain-specific nuances.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`

#### AI Mapping Feedback Loop

- [ ] `p3` - **ID**: `cpt-insightspec-fr-cn-ai-feedback-loop`

The system MUST capture human corrections to AI-generated mapping proposals (approved mappings, rejected proposals, manual overrides) and store them as a feedback dataset. This feedback MUST be available for improving future AI proposals — when a human repeatedly corrects the same type of mapping (e.g., `issue_status` → `task_state`), subsequent proposals for similar sources MUST reflect the learned pattern. The feedback dataset MUST be scoped per domain (not per connector) to maximize cross-source learning.

**Rationale**: Without a feedback loop, the AI mapping assistant makes the same mistakes repeatedly, eroding trust and adding review overhead. Capturing corrections transforms each onboarding into a training signal.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`

#### Silver Schema Evolution

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-silver-schema-evolution`

Canonical Silver schemas (`class_{domain}`) MUST be versioned independently from connector specifications. When a Silver schema changes (new required field, field type change, field removal), the system MUST: (1) validate all active unifier mappings against the new schema version before activation; (2) clearly report which connectors cannot populate newly required fields; (3) support a transition period where the new field is optional (with a default or NULL) until all relevant connectors have updated mappings. The system MUST NOT allow a Silver schema change that silently breaks existing unifier mappings. Backward-incompatible Silver schema changes MUST require a major version bump.

**Rationale**: With 20+ connectors mapping to the same Silver schema, a change to the canonical schema requires coordinated updates across all unifiers. Without versioning and compatibility validation, a schema evolution in Silver causes the same class of silent breakage that connector versioning prevents in Bronze.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`

### 5.4 Security & Access Control

#### Connector Configuration Authorization

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-config-authz`

Connector instance management (configure, activate, deactivate, delete) MUST be protected by the platform's Permission Architecture (see `docs/architecture/permissions/PERMISSION_PRD.md`). The connector framework MUST declare the required permission scopes and enforce them through the platform's Role + Explicit Scope Grant model. Credential provisioning MUST be restricted to the workspace in which the connector operates — no cross-workspace credential access.

**Rationale**: Connectors access external systems with customer credentials; unrestricted configuration creates a risk of unauthorized data collection or credential exposure.

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`

#### Silver Mapping Approval Rights

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-mapping-approval`

Silver layer field mapping activation MUST be gated by a dedicated permission scope defined in the Permission Architecture (see `docs/architecture/permissions/PERMISSION_PRD.md`). The connector framework MUST enforce that only users with the mapping approval scope can publish Silver mappings to production. The approval chain MUST be auditable.

**Rationale**: Silver mappings determine what data reaches analytics. Incorrect mappings can produce misleading metrics at scale; a review gate prevents unvetted semantic decisions from reaching production.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`, `cpt-insightspec-actor-cn-connector-author`

#### Connector Authorship Tier Permissions

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-authorship-tiers`

The system MUST enforce distinct permission boundaries for the three authorship tiers, using the platform's Permission Architecture (see `docs/architecture/permissions/PERMISSION_PRD.md`): (1) first-party authors MAY modify the framework and all connectors; (2) community authors MAY submit connectors for review but MUST NOT deploy without approval; (3) self-service authors MAY build workspace-scoped connectors but MUST NOT publish to the global catalog.

**Rationale**: Without tier-based permissions, community or self-service connectors could bypass quality controls or affect other workspaces.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`, `cpt-insightspec-actor-cn-connector-author`, `cpt-insightspec-actor-cn-workspace-admin`

#### Row-Level Security (RLS) on Bronze and Silver Tables

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-rls`

ClickHouse ROW POLICYs **MUST** be applied to all Bronze and Silver tables to enforce tenant-level data isolation at the database level. RLS policies **MUST** filter on `tenant_id` column and restrict each tenant role to only their data. Policies **MUST** survive table recreation (DROP + CREATE) — they are managed separately via `apply-rls.sh` and a declarative RLS config. The RLS config **MUST** be applied automatically during `./dev-up.sh` initialization and **MUST** be re-applicable without data loss.

**Rationale**: Application-level `WHERE tenant_id = ...` is insufficient as the sole isolation mechanism. Database-level RLS provides defense-in-depth and prevents accidental cross-tenant data exposure via ad-hoc queries or BI tools connecting directly to ClickHouse.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`

### 5.5 Data Classification & Privacy

#### Data Sensitivity Classification

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-data-classification`

Every connector specification MUST classify each collected field into one of the following sensitivity levels: (1) **Public metadata** — non-sensitive operational data (commit hashes, timestamps, status labels); (2) **Internal metadata** — organization-internal but non-personal data (project names, repository names, team identifiers); (3) **Personal data (PII)** — data attributable to an individual (names, emails, employee IDs, job titles, department); (4) **Sensitive content** — message bodies, document text, code content. The sensitivity classification MUST be declared in the connector specification and enforced at collection time.

**Rationale**: HR connectors (BambooHR, Workday, LDAP) collect employee names, emails, job titles, and organizational hierarchy — all PII under GDPR. Without explicit classification, there is no systematic basis for retention, access control, or data subject rights enforcement.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`, `cpt-insightspec-actor-cn-connector-author`

#### PII Retention and Deletion

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-pii-retention`

Bronze data containing PII fields MUST be subject to a configurable retention policy per workspace. When a data subject exercises the right to erasure, the system MUST support deletion or anonymization of that individual's PII across all Bronze tables where their data appears. The system MUST provide an audit trail of deletion requests and their execution.

**Rationale**: GDPR Article 17 (Right to erasure) requires the ability to delete personal data upon request. Without a retention and deletion mechanism, the platform exposes customers to regulatory non-compliance.

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`

#### Data Subject Access Support

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-data-subject-access`

The system MUST support identifying all Bronze records containing PII for a specific individual, to enable data subject access requests (GDPR Article 15). The identification MUST work across all connectors that collect PII for the given individual, using source-native identifiers and (where available) the canonical `person_id` mapping provided by the Identity Manager.

**Rationale**: GDPR Article 15 requires data controllers to provide individuals with access to their personal data. The connector framework must support this across all sources.

**Actors**: `cpt-insightspec-actor-cn-workspace-admin`, `cpt-insightspec-actor-cn-identity-manager`

### 5.6 Connector Authorship

#### Connector SDK

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cn-connector-sdk`

The system MUST provide a published SDK enabling external developers to build new connectors without access to the platform codebase. The SDK MUST include: a base connector contract, specification format documentation, example connectors, and local testing capabilities.

**Rationale**: The platform team cannot build connectors for every tool in every customer's stack; self-service and community authorship are essential for long-tail coverage.

**Actors**: `cpt-insightspec-actor-cn-connector-author`

#### Specification Versioning and Compatibility

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cn-schema-versioning`

Connector specifications MUST carry a version. Breaking schema changes (field removal, type changes, renames) MUST be detectable before deployment. The system MUST validate that downstream consumers (Silver unifiers) are compatible with the connector specification version.

**Rationale**: Schema changes that silently break Silver pipelines are discovered only when dashboards show incorrect data — often days later. Pre-deployment validation prevents this class of incident.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`, `cpt-insightspec-actor-cn-connector-author`

#### Connector Onboarding UI

- [ ] `p3` - **ID**: `cpt-insightspec-fr-cn-onboarding-ui`

The system SHOULD provide a user interface for reviewing, modifying, and approving Silver layer field mappings when onboarding a new connector. The UI MUST present AI-generated proposals alongside the actual Bronze data for validation.

**Rationale**: Silver mapping requires seeing real data; a UI that presents proposals alongside samples enables faster, more confident authorship decisions.

**Actors**: `cpt-insightspec-actor-cn-platform-engineer`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Rate Limit Compliance

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cn-rate-limit-compliance`

The system MUST NOT exceed source API declared rate limits under any circumstances. When rate limits are encountered, the system MUST apply backoff and retry strategies that respect the source's rate limit response headers.

**Threshold**: Zero rate-limit-induced API bans across all source integrations, measured monthly.

**Rationale**: Exceeding rate limits can result in API key revocation by the source provider, causing complete data collection failure for all workspaces using that source.

#### Resource Quotas per Collection Run

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cn-resource-quotas`

Each collection run MUST be subject to configurable resource limits: maximum memory consumption, maximum CPU time, and maximum temporary storage. When a collection run exceeds any limit, it MUST be terminated gracefully (checkpoint progress if possible) rather than allowed to consume unbounded resources. Default limits MUST be defined per connector tier (lightweight connectors vs heavy connectors like large Git monorepos). Workspace admins MUST be able to adjust limits within platform-defined bounds.

**Threshold**: No single collection run may consume more than the configured resource ceiling; resource exhaustion events (OOM, disk full) MUST be zero under normal operation.

**Rationale**: At 5,000+ concurrent connector instances, a single heavy connector (e.g., backfilling 1M commits from a monorepo) can saturate network bandwidth, exhaust disk with temporary files, or starve other connectors of CPU/memory — violating the error isolation guarantee.

#### Privacy by Default

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cn-privacy-by-default`

Content fields (message text, email body, document content) MUST NOT be collected unless explicitly declared in the connector specification and approved by the workspace admin. The system MUST default to collecting metadata only — no undeclared content fields may appear in Bronze tables.

**Threshold**: Zero instances of undeclared content field collection, verified by audit of connector specifications and Bronze table contents.

**Rationale**: Collecting message/email content without explicit consent creates legal liability (GDPR, corporate policy) and erodes customer trust. The platform's value proposition is behavioral analytics, not content surveillance.

#### Data Freshness

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cn-data-freshness`

Bronze data for active connectors MUST be no older than the configured collection interval plus a tolerance of 1 collection cycle. If a connector misses two consecutive scheduled runs, the system MUST generate an alert for the workspace admin.

**Threshold**: Data freshness within 2x configured collection interval for 99% of active connector instances, measured weekly.

**Rationale**: Data analysts depend on timely data for operational dashboards; stale data without alerting leads to decisions based on outdated information.

#### Collection Pipeline Availability

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-cn-availability`

The connector framework MUST be available for scheduled collection runs at least 99.5% of the time, measured monthly. Planned maintenance windows MUST NOT exceed 30 minutes and MUST be scheduled during off-peak hours.

**Threshold**: 99.5% monthly uptime for the collection pipeline (excluding source API outages).

**Rationale**: Connector downtime directly impacts data freshness across all workspaces; extended outages create data gaps that may not be recoverable for sources without historical API access.

#### Bronze Data Recoverability

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-cn-recoverability`

Bronze data MUST be recoverable to a point no older than 24 hours (RPO ≤ 24h). Recovery of the collection pipeline to operational state MUST complete within 4 hours (RTO ≤ 4h). For sources that support historical data retrieval, the system SHOULD support backfilling gaps caused by outages.

**Threshold**: RPO ≤ 24 hours; RTO ≤ 4 hours.

**Rationale**: Bronze is the system of record for all collected data; loss beyond 24 hours may not be recoverable from source APIs due to retention limits on the source side.

#### Connector Observability

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-cn-observability`

Every connector instance MUST expose a health check endpoint indicating its operational status (healthy, degraded, unhealthy). The system MUST export structured, machine-readable metrics for each connector instance, including at minimum: total API calls, records processed per run, rate limit encounters, error counts by type, extraction latency, and resource consumption (memory, CPU). Metrics MUST be exportable in a standard format consumable by external monitoring systems. Metrics MUST support aggregation across dimensions: per connector type, per workspace, per source instance.

**Threshold**: 100% of active connector instances report health status and export metrics; metric latency ≤ 30 seconds from event to availability in monitoring.

**Rationale**: At 5,000+ connector instances, log-based auditing alone is insufficient for operational visibility. Structured metrics enable proactive alerting (e.g., gradual increase in rate limit hits before a ban), capacity planning (resource consumption trends), and SLA reporting (data freshness per workspace).

### 6.2 NFR Exclusions

- **Accessibility** (UX-PRD-002): Not applicable — the Connector Framework is a backend data pipeline with no end-user interface. The Connector Onboarding UI referenced in `cpt-insightspec-fr-cn-onboarding-ui` is a separate UI component with its own accessibility requirements.
- **Internationalization** (UX-PRD-003): Not applicable — internal platform component; data schemas are language-agnostic; admin interfaces use English only for v1.0.
- **Safety** (SAFE-PRD-001/002): Not applicable — pure information processing system with no physical interaction or potential for harm to people/property/environment.
- **Offline capability** (UX-PRD-004): Not applicable — server-side pipeline requiring network connectivity to source APIs by definition.
- **Inclusivity** (UX-PRD-005): Not applicable — narrow technical audience (platform engineers, connector authors); no public-facing user interface in this domain.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Connector SDK

- [ ] `p1` - **ID**: `cpt-insightspec-interface-cn-connector-sdk`

**Type**: Library / SDK

**Stability**: stable

**Description**: The public SDK surface for connector authors. Includes the base connector contract, connector specification format, and local testing harness. This is the primary integration point for community and self-service connector development.

**Breaking Change Policy**: Major version bump required for any changes to the base contract or specification format that would break existing connectors.

### 7.2 External Integration Contracts

#### Source System APIs

- [ ] `p1` - **ID**: `cpt-insightspec-contract-cn-source-api`

**Direction**: required from external system

**Protocol/Format**: HTTP/REST, GraphQL, or source-native protocol — varies per source

**Compatibility**: Each connector adapts to its source API version; connector specifications declare the minimum supported API version.

#### Bronze-to-Silver Data Contract

- [ ] `p2` - **ID**: `cpt-insightspec-contract-cn-bronze-to-silver`

**Direction**: provided by connector framework to Unifier/Silver layer

**Protocol/Format**: Bronze table schemas as defined by connector specifications

**Compatibility**: Connector specification versioning ensures Silver unifiers can validate compatibility before processing. Breaking changes in Bronze schema require corresponding unifier updates.

## 8. Use Cases

### 8.1 Connector Development

#### Develop a New First-Party Connector

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-cn-develop-connector`

**Actor**: `cpt-insightspec-actor-cn-platform-engineer`

**Preconditions**:
- Source API documentation is available
- Domain unifier schema exists for the source's domain (or will be created)

**Main Flow**:
1. Platform engineer creates a connector specification declaring source identity, authentication, endpoints, and entity schemas
2. Framework validates the specification for completeness and consistency
3. Engineer implements source-specific extraction logic using the SDK base contract
4. Engineer defines Silver layer field mappings from Bronze to the domain unifier schema
5. Engineer tests the connector locally against the source API
6. Framework validates that the Bronze output conforms to the declared specification
7. Connector is registered in the platform and available for workspace configuration

**Postconditions**: Connector is available for workspace admins to configure and activate.

**Alternative Flows**:
- **Specification validation fails (step 2)**: Framework reports specific validation errors; engineer corrects specification and retries
- **Source API requires unsupported auth pattern**: Engineer requests framework extension; connector development blocked until supported

#### Build a Community Connector

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-cn-community-connector`

**Actor**: `cpt-insightspec-actor-cn-connector-author`

**Preconditions**:
- Connector SDK is published and accessible
- Author has access to the target source API

**Main Flow**:
1. Author installs the SDK and reviews documentation and example connectors
2. Author creates a connector specification for the target source
3. Author implements source-specific extraction logic extending the base contract
4. Author tests locally using the SDK testing harness
5. Author submits the connector for review by the platform team
6. Platform engineer reviews specification, Silver mapping proposals, and test results
7. Upon approval, connector is published and available to all workspaces

**Postconditions**: Community connector is available in the connector catalog.

**Alternative Flows**:
- **Review identifies issues (step 6)**: Platform engineer provides feedback; author revises and resubmits
- **Source requires Silver mapping for a new domain**: Platform engineer creates the domain unifier schema before the connector can be fully onboarded

### 8.2 Connector Operations

#### Configure and Monitor a Connector Instance

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-cn-configure-monitor`

**Actor**: `cpt-insightspec-actor-cn-workspace-admin`

**Preconditions**:
- Connector is registered in the platform
- Admin has workspace credentials for the target source instance

**Main Flow**:
1. Admin selects a connector from the available catalog
2. Admin provides source instance credentials and configuration (instance URL, API key, scope)
3. System validates credentials by attempting a test connection
4. Admin activates the connector instance
5. System begins collecting data per the configured schedule (managed by Orchestrator)
6. Admin monitors collection runs — views status, record counts, errors, and data freshness

**Postconditions**: Data from the source instance flows into Bronze and is available for Silver unification.

**Alternative Flows**:
- **Credential validation fails (step 3)**: System reports the error; admin corrects credentials
- **Collection run fails (step 5)**: System logs error details in collection run audit; admin is notified; system retries on next scheduled run
- **Source API becomes unavailable**: System records consecutive failures; admin receives alert; other connectors continue unaffected

## 9. Acceptance Criteria

- [ ] A new Bronze-layer connector can be developed and tested by a platform engineer using only the SDK and connector specification, without modifying framework code
- [ ] An external developer can build, test, and submit a connector using the published SDK documentation alone
- [ ] Connector failures are isolated — killing one connector mid-run does not affect others
- [ ] Re-running a connector for the same data range produces identical Bronze output
- [ ] Schema changes in a connector specification are detected and validated before deployment; incompatible changes are blocked
- [ ] Collection run audit records are complete and queryable for all connector executions
- [ ] No content fields appear in Bronze tables unless explicitly declared in the connector specification
- [ ] Silver unifier produces identical output for equivalent data from different sources of the same domain
- [ ] Every connector specification classifies all collected fields by data sensitivity level
- [ ] PII deletion requests are executed across all relevant Bronze tables and produce an audit trail
- [ ] Data freshness alerts fire when a connector misses two consecutive scheduled runs
- [ ] Bronze data can be recovered to within 24 hours of the failure point
- [ ] A new connector instance with a 30-day fast-start window begins producing analytics data within hours, while backfilling remaining history in the background
- [ ] A backfill interrupted mid-run resumes from its last checkpoint without re-downloading already collected data
- [ ] A Silver schema change that adds a required field is blocked until all active unifiers can populate it (or a default is defined)
- [ ] No single collection run exceeds its configured resource limits; exceeding runs are terminated gracefully
- [ ] All active connector instances expose health status and export structured metrics to the monitoring system
- [ ] Records that fail Silver unification are quarantined (not dropped) and the quarantine count is visible as a data quality metric
- [ ] Incremental sync runs are never starved by concurrent backfill runs, even under platform-wide throughput saturation
- [ ] Bronze data older than the configured retention period is archived or deleted, and workspace admins are notified before deletion
- [ ] Any Silver record can be traced back to its originating Bronze record(s) via the lineage reference

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Connector Orchestrator | Schedules and triggers connector execution runs (see `docs/components/connectors-orchestrator/specs/PRD.md`) | p1 |
| Identity Manager | Internal Insight platform service providing `person_id` mapping for source-native user IDs; Silver data can be joined with this mapping at query time or in Gold (see `docs/domain/identity-resolution/`) | p2 |
| Source System APIs | External SaaS APIs providing data; availability and rate limits vary per source | p1 |
| Credential/Secrets Management | Secure storage and retrieval of source API credentials per workspace | p1 |
| Data Catalog / Semantic Dictionary | Downstream consumer of semantic metadata from Silver field definitions | p2 |
| Permission Architecture | Defines role-based access control for connector configuration and Silver mapping approval (see `docs/architecture/permissions/`) | p1 |

## 11. Assumptions

- Source APIs provide stable, documented interfaces with backwards-compatible versioning
- Source APIs support some form of incremental data retrieval (timestamp-based, cursor-based, or event-based)
- Workspace admins are responsible for providing valid credentials for their source instances
- The Medallion Architecture (Bronze → Silver → Gold) is the established data pipeline pattern and will not change during v1.0
- Identity resolution is handled by the Identity Manager (a separate Insight platform service) — connectors provide source-native user identifiers; Silver tables retain these source-native IDs and are NOT enriched with `person_id`; the mapping is available for joining at query time or in Gold
- Each workspace may have multiple instances of the same source type (e.g., two Jira instances)
- Customers deploying HR connectors are aware that employee PII will be collected and have obtained necessary consent or legal basis
- The platform operates as a data processor (GDPR Article 28) — customers (data controllers) are responsible for legal basis of collection; the framework provides the technical mechanisms for retention, deletion, and access

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Source API breaking changes | Connector stops collecting data; Bronze schema mismatch breaks Silver pipeline | Connector specification versioning with compatibility validation; monitoring alerts on consecutive failures |
| Rate limit exhaustion | Source provider revokes API access; data collection halted for all workspaces using that source | Framework-enforced rate limit compliance; per-source backoff strategies; configurable collection frequency |
| Custom fields explosion | Thousands of unique custom fields across customers degrade query performance and storage | Bounded custom field storage pattern; workspace-scoped custom field discovery |
| Silver mapping drift | Unified schemas diverge from source reality as sources add/change fields over time | Periodic Silver mapping validation against actual Bronze data; alerting on unmapped fields |
| Community connector quality | Poorly implemented connectors cause data quality issues or source API abuse | Mandatory review process; SDK testing harness with validation suite; connector certification tiers |
| Credential security | Leaked source API credentials expose customer data | Credentials managed by dedicated secrets infrastructure (out of scope); connectors never access raw credentials |
| Backfill rate limit exhaustion | Historical data loading for a new large customer consumes all API quota, blocking incremental collection for existing workspaces | Two-phase backfill (fast-start + background); backfill runs share rate limit budget with incremental runs; configurable priority |
| Silver schema migration cascade | Adding a required field to a Silver schema requires simultaneous update of all connectors mapping to that domain | Silver schema versioning with transition periods; new required fields start as optional with defaults until unifiers are updated |
| Resource starvation by heavy connectors | One connector backfilling a million records consumes all available memory/disk/bandwidth | Per-run resource quotas with graceful termination; tiered default limits (lightweight vs heavy connectors) |
| Observability blind spots at scale | With 5,000+ instances, log-only monitoring misses gradual degradation (creeping latency, approaching rate limits) | Structured metrics export; health checks; alerting on trends, not just thresholds |
| Silent data loss at Silver boundary | Records failing unification are dropped; analysts see correct-looking but incomplete data | Silver quarantine mechanism; quarantine rate as a data quality metric; re-processing after mapping fix |
| Unbounded Bronze storage growth | At 200 GB/month, Bronze storage costs grow linearly with no upper bound; most old Bronze data is never queried | Configurable retention policies; automatic archival to cold storage; notification before deletion |
| Backfill starving incremental sync | Platform-wide throughput saturation during large backfills delays real-time data freshness | Global priority: incremental sync always yields before backfill; backfill throttles down under contention |
| GDPR non-compliance | PII collected by HR connectors without proper retention/deletion leads to regulatory fines | Data sensitivity classification per field; configurable retention policies; deletion/anonymization capability for data subject requests |
| Data freshness degradation | Silent connector failures lead to stale analytics and incorrect business decisions | Consecutive failure alerting; collection run audit trail; data freshness monitoring per connector instance |
| Bronze data loss | Unrecoverable Bronze data loss requires full re-ingestion from source APIs (some sources have limited historical access) | RPO ≤ 24h backup strategy; backfill capability for sources with historical API access |
