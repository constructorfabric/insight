---
status: draft
date: 2026-09-07
---

> [!NOTE]
> Keep this DESIGN very brief — short sections, no filler.

# DESIGN -- Insight v3 Core Service

- [ ] `p3` - **ID**: `cpt-insightspec-v3-design-core`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional context](#4-additional-context)
  - [Test strategy](#test-strategy)
  - [Open questions](#open-questions)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

Build on gears as much as possible: where a gear covers a capability, compose it instead of writing our own.

### 1.2 Architecture Drivers

TBD

#### Functional Drivers

Functional requirements and their IDs are in [PRD §5](./PRD.md#5-functional-requirements). Design responses: TBD.

#### Architecture Decision Records

| ADR | Decision |
|-----|----------|
| `cpt-insightspec-v3-adr-separate-service` | A separate service rather than extending analytics |
| `cpt-insightspec-v3-adr-static-ingest-token` | A static per-environment ingest token rather than the platform OIDC path |
| `cpt-insightspec-v3-adr-frontend-left-untouched` | The existing frontend is left alone; the new pages sit beside it |
| `cpt-insightspec-v3-adr-schema-driven-widgets` | Widgets are described by a schema, not written as code |

#### NFR Allocation

| NFR | Design response |
|-----|-----------------|
| `cpt-insightspec-v3-nfr-efficiency` | TBD |
| `cpt-insightspec-v3-nfr-reliability` | TBD |
| `cpt-insightspec-v3-nfr-performance` | TBD |
| `cpt-insightspec-v3-nfr-security` | TBD |
| `cpt-insightspec-v3-nfr-versatility` | TBD |

### 1.3 Architecture Layers

| Layer | Responsibility | Technology |
|-------|---------------|------------|
| Presentation | Any page renders widgets from a JSON schema | React 19 + TypeScript |
| Application | Raw-data ingest and read; metric, widget and dashboard definitions | Rust service `insight-v3-core` |
| Infrastructure | Raw-data storage | ClickHouse |

## 2. Principles & Constraints

### 2.1 Design Principles

#### Gears First

- [ ] `p2` - **ID**: `cpt-insightspec-v3-principle-gears-first`

Where a gear covers a capability, compose it instead of writing our own.

#### Widgets Are Schema, Not Code

- [ ] `p1` - **ID**: `cpt-insightspec-v3-principle-schema-driven`

Widgets are described by a JSON schema and the frontend renders from that schema, so a new widget type is a schema change.

#### Ingest Does Not Know the Shape

- [ ] `p1` - **ID**: `cpt-insightspec-v3-principle-shape-agnostic-ingest`

The ingest side accepts data without knowing its shape; interpretation happens on read.

### 2.2 Constraints

#### TBD

- [ ] `p1` - **ID**: `cpt-insightspec-v3-constraint-tbd`

TBD



## 3. Technical Architecture

### 3.1 Domain Model

- Raw record — arbitrary JSON under a logical `table_name`.
- Table — a logical name for a set of records, with optionally declared columns and their types.
- Metric — a named calculation over raw data.
- Widget — any visual representation of data over chosen columns.
- Dashboard — a named, addressable page arranging widgets.
- Alert — a condition on a metric plus where to send it when it fires.

### 3.2 Component Model

#### insight-v3-core Service

- [ ] `p1` - **ID**: `cpt-insightspec-v3-component-core-service`

##### Why this component exists

TBD

##### Responsibility scope

Ingests raw data, serves reads, and owns metric, widget, dashboard and alert definitions.

##### Responsibility boundaries

TBD

##### Related components (by ID)

- `cpt-insightspec-v3-component-ai-chat` — serves it the same contracts the UI uses

#### AI Chat

- [ ] `p2` - **ID**: `cpt-insightspec-v3-component-ai-chat`

##### Why this component exists

TBD

##### Responsibility scope

Answers questions, and creates metrics, widgets, dashboards and alerts.

##### Responsibility boundaries

TBD

##### Related components (by ID)

- `cpt-insightspec-v3-component-core-service` — reads and writes definitions through it

### 3.3 API Contracts

The contract is the OpenAPI spec the service serves and commits, not a list in this file.

**Technology**: OpenAPI, served by the api-gateway gear

**Location**: `docs/components/backend/insight-v3-core/openapi.json`, drift-checked against the emitted document by the service's `committed_openapi_document_is_current` test.

### 3.4 Internal Dependencies

- Usage recording and reporting already exist in the analytics service: [src/backend/services/analytics/src/api/usage.rs](../../../../src/backend/services/analytics/src/api/usage.rs). Its summary reports opens and people per event target, per page and per person.

### 3.5 External Dependencies

- ClickHouse — raw-data storage.

### 3.6 Interactions & Sequences

#### TBD

**ID**: `cpt-insightspec-v3-seq-tbd`

TBD

### 3.7 Database schemas & tables

- [ ] `p1` - **ID**: `cpt-insightspec-v3-db-clickhouse`

#### Table: raw_data

**ID**: `cpt-insightspec-v3-dbtable-raw-data`

The schema lives in the migration, not here: [src/backend/services/insight-v3-core/src/migration.rs](../../../../src/backend/services/insight-v3-core/src/migration.rs).

`raw_data` is the stream the migration brings up. A further ingest stream gets
a physical table of its own with the identical columns, requested by an
administrator through `PUT /v1/tables/{table}` — see
[ADR-0006](./ADR/0006-a-table-per-ingest-stream.md).

### 3.8 Deployment Topology

TBD


## 4. Additional context

### Test strategy

Five families, matching what Insight already runs in CI:

- Unit tests per crate and per frontend package, line coverage above 80%.
- API contract tests against the OpenAPI spec, covering the status codes each endpoint declares.
- Metric tests — one per built-in metric definition, as today. Metrics created at runtime are validated when created; there is no test per user-created metric.
- End-to-end UI journeys against the deployed stand.
- Dependency, code, secret and image scans on every pull request.

Metrics come from three sources: the dev stand during regression, our own usage, and customer stands that opt in.

### Open questions

- Can a customer bring their own AI subscription — a team plan of their own — instead of using ours?

## 5. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **DECOMPOSITION**: [DECOMPOSITION.md](./DECOMPOSITION.md)
- **ADRs**: [ADR/](./ADR/)
