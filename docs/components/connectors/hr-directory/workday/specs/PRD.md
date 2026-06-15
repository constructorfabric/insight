# PRD — Workday Connector

> Version 1.0 — June 2026
> Based on: HR Directory domain (`docs/components/connectors/hr-directory/README.md`), BambooHR connector PRD (`../../bamboohr/specs/PRD.md`)

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
  - [5.1 Data Collection](#51-data-collection)
  - [5.2 Data Integrity](#52-data-integrity)
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
- [13. Open Questions](#13-open-questions)
  - [OQ-WD-1: Incremental extraction via Last Functionally Updated prompt](#oq-wd-1-incremental-extraction-via-last-functionally-updated-prompt)
  - [OQ-WD-2: BambooHR + Workday coexistence](#oq-wd-2-bamboohr--workday-coexistence)
  - [OQ-WD-3: Native effective-dated history (transaction log)](#oq-wd-3-native-effective-dated-history-transaction-log)
  - [OQ-WD-4: Department / division mapping per instance](#oq-wd-4-department--division-mapping-per-instance)

<!-- /toc -->

---

## 1. Overview

### 1.1 Purpose

The Workday connector extracts HR directory data — worker records and time-off requests — from Workday RaaS (Reports-as-a-Service) custom reports into the Insight platform's Bronze layer. This data feeds identity resolution (canonical `person_id` via work email), org hierarchy construction, and leave analytics.

### 1.2 Background / Problem Statement

Workday is the dominant enterprise HRIS (5000+ employees). The Insight platform requires HR directory data for:

1. **Identity resolution** — mapping source-system user identifiers (GitHub login, Jira account ID, etc.) to real people via work email as the canonical identity anchor.
2. **Org hierarchy** — enabling team-level aggregation of engineering metrics by supervisory organization and manager chain.
3. **Leave analytics** — time-off patterns feed burnout risk signals and availability forecasting.

Unlike BambooHR, Workday has **no fixed bulk-extraction endpoint** with a guaranteed field set. The extraction mechanism is RaaS: the customer builds custom reports in Workday Report Writer following an Insight-defined **report contract** (column set + XML aliases + prompts), enables them as web services, and shares them with an Integration System User (ISU). The connector fetches the reports as JSON. The field set is therefore controlled on the Workday side, and the connector must validate the configured reports against the contract.

Although Workday records are natively effective-dated, the v1 connector extracts **current state only** (the report returns as-of-today values) — exactly like BambooHR. Historical change tracking is provided by the Silver-layer SCD2 snapshot chain (`snapshot` → `fields_history` → `identity_inputs`). Native history extraction is deferred (see OQ-WD-3).

### 1.3 Goals (Business Outcomes)

1. Enable identity resolution for all Insight workspaces using Workday as their HR system.
2. Provide org hierarchy data (supervisory organization, manager chain) for team-level metric scoping.
3. Collect time-off history for availability and burnout risk analytics.
4. Allow customer-specific report columns (custom/calculated fields) to flow into Bronze `raw_data` and be tracked by the Silver SCD2 chain without connector changes.

### 1.4 Glossary

| Term | Definition |
|------|-----------|
| **RaaS** | Reports-as-a-Service — Workday's mechanism for exposing a custom report as a web service endpoint (`/ccx/service/customreport2/{tenant}/{owner}/{report}`) |
| **ISU** | Integration System User — Workday service account used for API authentication; access is scoped via an Integration System Security Group and domain security policies |
| **Report contract** | The Insight-defined specification of report columns, XML aliases, and prompts that the customer's custom reports must follow |
| **XML alias** | The explicit column alias set in Report Writer; becomes the JSON field name in the RaaS response. Auto-generated aliases differ per tenant — the contract requires explicit aliases |
| **Workday-delivered field** | A field shipped as part of Workday's standard worker data model — present in every tenant (e.g., Employee ID, Hire Date, Supervisory Organization) |
| **Identity key** | The field used for cross-system person resolution — `Work_Email` for Workday |

---

## 2. Actors

### 2.1 Human Actors

#### Platform Engineer

**ID**: `cpt-insightspec-actor-wd-platform-engineer`

Configures Workday connections (ISU credentials, base URL, report paths), monitors collection runs, and troubleshoots extraction failures.

#### Customer Workday Administrator

**ID**: `cpt-insightspec-actor-wd-customer-admin`

Builds the two custom reports per the Insight report contract, creates the ISU and security group, grants domain access, and shares the reports with the ISU. Owns the Workday side of the integration.

#### Data Analyst

**ID**: `cpt-insightspec-actor-wd-data-analyst`

Consumes Workday Bronze data through Silver/Gold layers for org hierarchy analysis, headcount reporting, and leave pattern analytics.

### 2.2 System Actors

#### Orchestrator

**ID**: `cpt-insightspec-actor-wd-orchestrator`

Triggers Workday connector runs on schedule and routes output to the destination.

#### Identity Manager

**ID**: `cpt-insightspec-actor-wd-identity-manager`

Consumes identity observations derived from `workers` (via the `fields_history` → `identity_inputs` chain) to maintain the canonical `person_id` mapping used by all Silver streams.

#### Destination (ClickHouse)

**ID**: `cpt-insightspec-actor-wd-destination`

Receives extracted records and writes them to Bronze tables.

---

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Workday API access requires an ISU whose security group has the domains needed by the report fields (at minimum: Worker Data: Public Worker Reports, Person Data: Work Email). **A missing domain does not fail the request — the field silently arrives empty.**
- RaaS runs the report on demand; response time and size scale with tenant headcount. Large tenants (50k+ workers) may approach RaaS execution timeouts on full pulls.
- There is **no public Workday developer tenant**. Development uses mock fixtures replaying the RaaS response shape; validation requires the customer's Sandbox tenant during onboarding.
- All API requests require HTTPS (mock endpoints for local development excepted).

---

## 4. Scope

### 4.1 In Scope

- Extraction of worker directory data via a customer-built RaaS report following the Insight report contract (Workday-delivered fields only in the standard column set).
- Extraction of time-off requests via a second RaaS report with `From_Date`/`To_Date` prompts.
- Full refresh sync on both streams (current-state extraction, BambooHR-style).
- Customer-specific extra report columns flowing into Bronze `raw_data` (key-value tracking via dbt var `workday_custom_fields`).
- `tenant_id`, `source_id`, `unique_key` injection on all records (platform invariants).
- Silver chain: bronze→RMT promotion, SCD2 snapshot, fields_history, identity_inputs, class_people / class_hr_events / class_hr_working_hours staging.

### 4.2 Out of Scope

- Incremental extraction via `Last_Functionally_Updated` report prompt — deferred (OQ-WD-1).
- Native effective-dated history / transaction-log extraction — deferred (OQ-WD-3).
- Supervisory organization hierarchy as a separate stream (org names are inline in worker records; hierarchy construction from manager chain is a Silver concern).
- Workday SOAP (WWS) and WQL access paths — RaaS only in v1.
- OAuth 2.0 authentication — ISU Basic auth is sufficient for RaaS (OAuth may be revisited if a customer mandates it).
- Silver/Gold layer transformations beyond the staging models shipped with the connector.
- Write operations (worker updates, time-off approval).

---

## 5. Functional Requirements

### 5.1 Data Collection

#### Worker Data Collection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-wd-collect-workers`

The system **MUST** extract worker records from the customer's workers RaaS report, collecting the contract's standard column set: worker identity (Employee ID, names, work email), org hierarchy (supervisory organization, manager ID/email), job (business title, job profile), classification (worker type, worker status), location (location, country, city), employment dates (hire, original hire, termination), last-modified timestamp (Last Functionally Updated), and scheduled weekly hours.

**Rationale**: Worker data is the foundation for identity resolution, org hierarchy, and all person-level analytics in the Insight platform.

**Actors**: `cpt-insightspec-actor-wd-orchestrator`, `cpt-insightspec-actor-wd-destination`

#### Leave Request Collection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-wd-collect-leave-requests`

The system **MUST** extract time-off requests from the customer's leave RaaS report, collecting: request ID, employee ID, time-off type, start date, end date, quantity, unit, status, and submission moment. The report's date range **MUST** be controlled via `From_Date`/`To_Date` web service prompts.

**Rationale**: Leave request data feeds burnout risk signals, availability forecasting, and team capacity analytics.

**Actors**: `cpt-insightspec-actor-wd-orchestrator`, `cpt-insightspec-actor-wd-destination`

#### Custom Column Passthrough

- [ ] `p2` - **ID**: `cpt-insightspec-fr-wd-custom-columns`

The system **MUST** preserve any extra columns present in the workers report (custom fields, calculated fields) in the Bronze `raw_data` column, without requiring connector changes. Tracking of specific custom columns at the Silver layer is configured via the dbt var `workday_custom_fields`.

**Rationale**: Workday has no field-discovery endpoint analogous to BambooHR `meta/fields`; the report definition IS the field registry. Extra columns must flow through without schema changes.

**Actors**: `cpt-insightspec-actor-wd-customer-admin`, `cpt-insightspec-actor-wd-destination`

### 5.2 Data Integrity

#### Report Contract Validation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-wd-report-contract`

The connector's `check` operation **MUST** fail when the configured workers report is unreachable or returns a payload without the `Report_Entry` wrapper. Documentation **MUST** specify the exact column aliases so that a mis-built report is detected at onboarding, not silently ingested with missing fields.

**Rationale**: With RaaS, nothing guarantees the field set — the customer's report definition does. A report built off-contract must fail loudly (fail-fast platform principle), because missing identity columns silently break person resolution downstream.

**Actors**: `cpt-insightspec-actor-wd-platform-engineer`, `cpt-insightspec-actor-wd-customer-admin`

#### Deduplication

- [ ] `p1` - **ID**: `cpt-insightspec-fr-wd-deduplication`

The system **MUST** define primary keys for each stream to enable deduplication at the destination:
- `workers`: `unique_key` = `{tenant_id}-{source_id}-{Employee_ID}`
- `leave_requests`: `unique_key` = `{tenant_id}-{source_id}-{Request_ID}`

**Rationale**: Primary keys enable ReplacingMergeTree deduplication at the destination, preventing duplicate records across collection runs.

**Actors**: `cpt-insightspec-actor-wd-destination`

#### Identity Key

- [ ] `p1` - **ID**: `cpt-insightspec-fr-wd-identity-key`

The system **MUST** collect `Work_Email` for every worker record. This field serves as the primary identity anchor for cross-system person resolution via the Identity Manager. The field may legitimately be empty for contingent workers; such records still carry `Employee_ID` as the source-account binding.

**Rationale**: Work email is the most reliable cross-system identifier for HR-to-engineering-tool person matching.

**Actors**: `cpt-insightspec-actor-wd-identity-manager`

#### Sync Mode

- [ ] `p1` - **ID**: `cpt-insightspec-fr-wd-full-refresh-sync`

Both streams use **full refresh** sync mode. The RaaS reports return current-state values on every call; there is no built-in delta mechanism. Leave requests use a date-range filter (`workday_start_date` to current date) via report prompts. The `Last_Functionally_Updated` column is collected to enable future incremental extraction (OQ-WD-1).

**Rationale**: Full refresh is simple, reliable, and identical to the proven BambooHR pattern, including the downstream SCD2 snapshot chain. Incremental extraction requires additional customer-side report setup (a prompt-enabled filter) and is deferred until tenant scale demands it.

**Actors**: `cpt-insightspec-actor-wd-orchestrator`

#### Fault Tolerance

- [ ] `p1` - **ID**: `cpt-insightspec-fr-wd-fault-tolerance`

The system **MUST** handle transient API failures with retry and backoff, and fail clearly on authentication errors (401/403) without retry.

**Rationale**: RaaS report execution can fail transiently under tenant load. Robust retry ensures collection completes; auth failures must surface immediately as configuration errors.

**Actors**: `cpt-insightspec-actor-wd-orchestrator`

---

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-wd-auth-isu-basic`

The connector **MUST** support ISU Basic authentication. ISU credentials, RaaS base URL, and both report paths **MUST** be configurable via the source connection specification (not hardcoded) and **MUST** be required fields with no defaults.

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-wd-schema-compliance`

All Bronze records **MUST** use the report contract's column aliases as field names (Workday-style `Snake_Case`) with no renaming. Schema transformations occur at the Silver layer.

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-wd-idempotent-writes`

Re-running the connector **MUST** produce records that deduplicate cleanly at the destination via `unique_key`. The connector performs no writes — idempotency is ensured by deterministic report output and primary-key deduplication.

### 6.2 NFR Exclusions

- **Performance SLAs**: Not applicable — RaaS report execution time depends on the customer's tenant size and Workday's infrastructure.
- **High availability**: The connector runs as a scheduled batch job; no real-time availability requirement.
- **Data encryption at rest**: Handled by the destination (ClickHouse) infrastructure, not the connector.

---

## 7. Public Library Interfaces

### 7.1 Public API Surface

Not applicable. The Workday connector is a declarative manifest (YAML) executed by the Airbyte Declarative Connector framework. It does not expose a public API.

### 7.2 External Integration Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-contract-wd-raas`

**Workday RaaS** — the connector consumes two customer-built custom reports exposed as web services. The report contract (columns, aliases, prompts) and API details are specified in the [DESIGN](./DESIGN.md) §3.3 and the connector README.

---

## 8. Use Cases

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-wd-onboarding`

**UC-1: Customer Onboarding**

**Trigger**: A customer wants Workday as their HR source.

**Flow**:
1. Insight provides the report contract (column aliases, prompts, ISU domain checklist) to the customer's Workday administrator.
2. The administrator creates the ISU + security group, builds both reports, enables them as web services, and shares them with the ISU.
3. Platform engineer configures the K8s Secret (base URL, ISU credentials, report paths).
4. The connector `check` validates the workers report end-to-end against the customer's Sandbox, then Production tenant.

**Postcondition**: Connection is validated; scheduled syncs can start.

---

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-wd-scheduled-sync`

**UC-2: Scheduled Full Refresh**

**Trigger**: Orchestrator triggers a scheduled run.

**Flow**:
1. Connector fetches all workers via the workers report (`?format=json`).
2. Connector fetches time-off requests via the leave report (`From_Date`/`To_Date` prompts).
3. All records are written to Bronze tables. Destination deduplicates on `unique_key`.
4. dbt runs the Silver chain: snapshot detects changed workers, fields_history emits per-field transitions, identity_inputs feeds the Identity Manager.

**Postcondition**: Bronze reflects the current state; SCD2 history is extended with detected changes.

---

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-wd-identity-feed`

**UC-3: Identity Manager Feed**

**Trigger**: Fresh worker records land in the `workers` Bronze table and dbt runs.

**Flow**:
1. `workday__workers_snapshot` appends SCD2 versions for changed workers.
2. `workday__workers_fields_history` derives per-field change rows.
3. `workday__identity_inputs` emits UPSERT/DELETE identity observations (email, employee_id, names, department, manager, status).
4. Identity Manager resolves observations to canonical `person_id`.

**Postcondition**: All Workday workers have a canonical `person_id` usable by all Silver streams.

---

## 9. Acceptance Criteria

1. The connector successfully extracts worker records from a contract-conformant RaaS report (mock fixture or customer Sandbox) and writes them to the destination.
2. The connector successfully extracts time-off requests within the configured date range via report prompts.
3. Full refresh sync correctly fetches all workers and all in-range leave requests on every run.
4. Extra report columns appear in Bronze `raw_data` without connector changes.
5. The connector fails gracefully on 401/403 with a clear error message; `check` fails on an unreachable or off-contract report.
6. All records include `tenant_id`, `source_id`, and `unique_key`.
7. The dbt chain (promotion → snapshot → fields_history → identity_inputs → class staging models) builds without errors and passes schema tests.

---

## 10. Dependencies

| Dependency | Type | Purpose |
|-----------|------|---------|
| Workday RaaS (customreport2) | External | Source system API |
| Customer-built custom reports | External | Extraction contract — owned by the customer's Workday administrator |
| Airbyte Declarative Connector framework (CDK v7.0+) | Runtime | Connector execution engine |
| ClickHouse destination connector | Runtime | Bronze table writes |
| dbt macros `snapshot`, `fields_history`, `identity_inputs_from_history`, `promote_bronze_to_rmt` | Internal | Silver SCD2 chain |
| Identity Manager | Downstream | Consumes identity observations for person resolution |

---

## 11. Assumptions

1. The customer can build and maintain the two custom reports per the Insight report contract (standard Workday Report Writer capability).
2. The ISU security group grants the domains required by the report fields; the onboarding checklist covers domain verification.
3. Workday-delivered fields in the contract (Employee ID, names, hire/termination dates, worker status/type, supervisory organization, manager chain, location, scheduled weekly hours, Last Functionally Updated) exist in every Workday tenant.
4. Tenant headcount is small enough for full-pull RaaS execution within timeouts; incremental extraction (OQ-WD-1) is the planned mitigation for larger tenants.
5. `Last Functionally Updated` is updated by Workday whenever any worker field changes.

---

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Customer builds the report off-contract (wrong aliases, missing columns) | Missing/empty Bronze fields; broken identity resolution | Explicit alias requirements in the contract; `check` validation at onboarding; dbt `not_null` tests fail loudly |
| ISU lacks a security domain — fields silently arrive empty | Silent data gaps (worse than errors) | ISU domain checklist in onboarding docs; post-onboarding data completeness review |
| RaaS timeout on very large tenants | Failed or partial collection | Move to incremental extraction via `Last_Functionally_Updated` prompt (OQ-WD-1) |
| Customer edits the report (renames/removes columns) after onboarding | Schema drift, missing fields | Report named `Insight_*` and documented as Insight-owned; dbt tests detect missing data |
| No Workday tenant available during development | Cannot test against real API | Mock fixtures replaying `Report_Entry` shape; contract validated in customer Sandbox at onboarding |

---

## 13. Open Questions

### OQ-WD-1: Incremental extraction via Last Functionally Updated prompt

RaaS has no built-in delta mechanism, but the workers report can define a prompt-enabled filter on `Last Functionally Updated`, turning the cursor into a URL parameter (`?Last_Functionally_Updated=...`). With incremental extraction, every fetched row is by definition a new version — the dbt `snapshot()` change-detection step becomes redundant and Bronze can store versions directly (cursor must be the entry moment, NOT effective date, to survive retroactive transactions).

- At what tenant scale does full pull become impractical (RaaS timeout)?
- Should the prompt filter be part of the v1 report contract (unused until needed) to avoid a second customer-side change later?

### OQ-WD-2: BambooHR + Workday coexistence

Some clients may use both BambooHR and Workday (e.g. Workday for enterprise HR, BambooHR for a subsidiary). When both HR sources are active:

- Does the Identity Manager merge them by email?
- Which source takes precedence for org hierarchy (manager chain)?
- Should `class_people` carry a `source` field indicating which HR system is authoritative per person?

### OQ-WD-3: Native effective-dated history (transaction log)

Workday records are natively effective-dated; full history — including changes made before the connector was installed — can be extracted via transaction-log data sources (SOAP `Get_Workers` with `Transaction_Log_Criteria` or a transaction-based report). This would backfill `fields_history` with true historical transitions instead of poll-time snapshots.

- Is pre-onboarding org history valuable enough to justify the extra report/SOAP complexity?
- The current architecture supports it as a backfill of versioned rows — no redesign required.

### OQ-WD-4: Department / division mapping per instance

Workday has no standard `department`/`division` fields — supervisory organization is the only guaranteed org unit. Customers may model departments as custom organization types, cost centers, or levels in the supervisory hierarchy.

- v1 maps `Supervisory_Organization` → `department` uniformly. Should the mapping be configurable per customer (e.g. extra contract columns `Department`/`Division` filled from customer-specific org types)?
