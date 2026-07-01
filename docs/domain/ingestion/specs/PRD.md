---
status: proposed
date: 2026-03-23
---

# PRD — Ingestion Layer




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
  - [5.1 Connector Execution](#51-connector-execution)
  - [5.2 Orchestration](#52-orchestration)
  - [5.3 Bronze Layer](#53-bronze-layer)
  - [5.4 Silver Step 1 Transformations](#54-silver-step-1-transformations)
  - [5.5 Infrastructure Management](#55-infrastructure-management)
  - [5.6 Connector Packaging](#56-connector-packaging)
  - [5.7 Security](#57-security)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Use Case 1: Create a New Nocode Connector Package](#use-case-1-create-a-new-nocode-connector-package)
  - [Use Case 2: Scheduled Ingestion Pipeline Run](#use-case-2-scheduled-ingestion-pipeline-run)
  - [Use Case 3: Local Connector Debugging](#use-case-3-local-connector-debugging)
  - [Use Case 4: Add Data Source to Workspace](#use-case-4-add-data-source-to-workspace)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

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
  - [5.1 Connector Execution](#51-connector-execution)
  - [5.2 Orchestration](#52-orchestration)
  - [5.3 Bronze Layer](#53-bronze-layer)
  - [5.4 Silver Step 1 Transformations](#54-silver-step-1-transformations)
  - [5.5 Infrastructure Management](#55-infrastructure-management)
  - [5.6 Connector Packaging](#56-connector-packaging)
  - [5.7 Security](#57-security)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [Use Case 1: Create a New Nocode Connector Package](#use-case-1-create-a-new-nocode-connector-package)
  - [Use Case 2: Scheduled Ingestion Pipeline Run](#use-case-2-scheduled-ingestion-pipeline-run)
  - [Use Case 3: Local Connector Debugging](#use-case-3-local-connector-debugging)
  - [Use Case 4: Add Data Source to Workspace](#use-case-4-add-data-source-to-workspace)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)



## 1. Overview

### 1.1 Purpose

The Ingestion Layer provides the end-to-end data pipeline from external source APIs to unified Silver step 1 tables. It replaces the previously designed custom Orchestrator ([Orchestrator PRD](../../../components/orchestrator/specs/PRD.md)) and custom Connector Framework ([Connector Framework PRD](../../connector/specs/PRD.md)) with an industry-standard stack: Airbyte for data extraction, Argo Workflows for orchestration, and dbt-clickhouse for Bronze-to-Silver transformations. Everything runs in Kubernetes (Kind for local development).

### 1.2 Background / Problem Statement

The previous approach relied on a custom stdout JSON protocol for connectors, a custom runner for execution, and a custom orchestrator for scheduling. While architecturally sound, this approach required significant engineering investment to build and maintain. The ingestion layer adopts proven open-source tools — Airbyte provides 300+ pre-built connectors and a declarative connector builder, Argo Workflows provides Kubernetes-native DAG orchestration, and dbt provides battle-tested SQL transformation framework. Connections are managed via Airbyte API (see `airbyte-toolkit/connect.sh`).

### 1.3 Goals (Business Outcomes)

- Reduce new connector development time: hours for nocode (declarative YAML), days for CDK (Python)
- Leverage Airbyte's existing connector ecosystem for common data sources
- Simplify orchestration with Kubernetes-native DAG workflows (Argo Workflows)
- Maintain workspace-level data isolation (`tenant_id` in every record)
- Enable self-contained Insight Connector packages (Airbyte manifest + dbt models + descriptor + credential template)
- Manage connections as code via Airbyte API and tenant YAML configs

### 1.4 Glossary


| Term                 | Definition                                                                                                       |
| -------------------- | ---------------------------------------------------------------------------------------------------------------- |
| Connector Package    | A directory containing connector definition (manifest or code), dbt transformation models, and a descriptor YAML |
| Declarative Manifest | An Airbyte YAML file defining a nocode connector: streams, authentication, pagination, and incremental sync      |
| CDK Connector        | A Python connector built using Airbyte's Connector Development Kit                                               |
| Argo Workflow        | A Kubernetes-native DAG workflow in Argo Workflows that orchestrates pipeline steps (Airbyte sync, dbt run)      |
| Insight Connector    | Complete pipeline package: Airbyte Connector + descriptor + dbt transformations + credential template            |
| Bronze Table         | Raw data table in ClickHouse where table names match Airbyte stream names directly, stored in a database namespace per tenant, preserving source-native schema |
| Silver Table         | Unified data table following `class_{domain}` naming, produced by dbt transformations from Bronze                |
| `tenant_id`          | Mandatory tenant isolation field present in every data record                                                    |
| Airbyte Connection   | A configured link between an Airbyte source and destination with stream selection and sync schedule              |
| Descriptor YAML      | A metadata file in a connector package declaring which Silver targets and streams the connector provides         |


## 2. Actors

### 2.1 Human Actors

#### Platform Engineer

**ID**: `cpt-insightspec-actor-ing-platform-engineer`

Deploys and maintains the ingestion infrastructure (Airbyte, Argo Workflows, ClickHouse, dbt) in Kubernetes. Manages tenant configurations and monitors pipeline health.

#### Connector Author

**ID**: `cpt-insightspec-actor-ing-connector-author`

Develops new connector packages (nocode manifests or CDK connectors) with corresponding dbt models and descriptor YAML. Tests connectors locally before production deployment.

#### Workspace Admin

**ID**: `cpt-insightspec-actor-ing-workspace-admin`

Configures data sources for their workspace. Provides credentials and connection parameters. Monitors data freshness and sync status for their workspace.

#### Data Analyst

**ID**: `cpt-insightspec-actor-ing-data-analyst`

Consumes Silver-layer unified tables for analysis. Reports data quality issues. Requests new data sources or transformations.

### 2.2 System Actors

#### Source API

**ID**: `cpt-insightspec-actor-ing-source-api`

External system from which data is extracted (GitHub API, Jira API, MS365 Graph API, etc.).

#### Airbyte Platform

**ID**: `cpt-insightspec-actor-ing-airbyte`

Manages connector execution, connection configuration, catalog discovery, and data delivery to destinations. Runs both nocode (declarative) and CDK (Python) connectors.

#### Argo Workflows Orchestrator

**ID**: `cpt-insightspec-actor-ing-argo`

Schedules and orchestrates ingestion pipelines via Kubernetes-native DAG workflows: triggers Airbyte syncs, waits for completion, triggers dbt transformations, handles retries. Selected over Kestra to eliminate PostgreSQL dependency — see [ADR-0002](ADR/0002-argo-over-kestra.md).

#### ClickHouse Cluster

**ID**: `cpt-insightspec-actor-ing-clickhouse`

Stores Bronze and Silver data in shard-local tables. Serves as the primary Airbyte destination and dbt execution target.

#### dbt-clickhouse

**ID**: `cpt-insightspec-actor-ing-dbt`

Executes SQL transformations from Bronze tables to Silver step 1 tables using the dbt-clickhouse adapter.

#### insight-toolbox

**ID**: `cpt-insightspec-actor-ing-toolbox`

Container image with all CLI tools (kubectl, helm, dbt, yq, Python) that runs init scripts and management operations inside the K8s cluster.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Production: Kubernetes cluster with Airbyte, Argo Workflows, and ClickHouse in separate namespaces, deployed via Helm charts
- Local development: Kind K8s cluster for full-stack debugging (Airbyte + Argo Workflows + ClickHouse + dbt)
- Ultra-light development: Nocode connector Docker images run directly without Airbyte platform (see [Airbyte Connector DESIGN](../../connector/specs/DESIGN.md))
- ClickHouse cluster with shard-local tables and ReplacingMergeTree engine
- MariaDB as alternative destination for specific use cases (described separately)

### 3.2 Expected Scale


| Dimension                  | Current | Projected |
| -------------------------- | ------- | --------- |
| Connector packages         | 20+     | 50+       |
| Concurrent Airbyte syncs   | 5-10    | 20-50     |
| Bronze tables              | 60+     | 150+      |
| Silver unified tables      | 15+     | 30+       |
| Records per sync (typical) | 1K-100K | 1K-1M     |
| Workspaces                 | 10+     | 100+      |


## 4. Scope

### 4.1 In Scope

- Data extraction from external sources via Airbyte (nocode and CDK connectors)
- Pipeline orchestration via Argo Workflows (scheduling, dependency management, retry, observability)
- Bronze layer storage in ClickHouse (table names match Airbyte stream names, stored in a configured database namespace)
- Silver step 1 transformations via dbt-clickhouse (`class_{domain}` tables)
- Connector package structure (manifest/code + dbt models + descriptor YAML)
- Airbyte connection management via API and tenant YAML configs
- Custom connector registration via Airbyte API
- `tenant_id` injection at connector level for tenant isolation
- Production deployment on Kubernetes via Helm charts
- Local development with Kind K8s cluster and ultra-light connector debugging

### 4.2 Out of Scope

- Silver step 2 (identity resolution) — covered by [Identity Resolution DESIGN](../../identity-resolution/specs/DESIGN.md)
- Gold layer metrics and aggregations — separate domain
- Custom Orchestrator — superseded by Argo Workflows (see [ADR-0002](ADR/0002-argo-over-kestra.md))
- Custom stdout JSON connector protocol — superseded by Airbyte Protocol
- External connector package registry — all packages in monorepo
- ClickHouse cluster administration and tuning — infrastructure concern
- Airbyte platform internals (Temporal, internal APIs) — treated as black box

## 5. Functional Requirements

### 5.1 Connector Execution

#### Airbyte Data Extraction

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-airbyte-extract`

The system **MUST** extract data from external source APIs via Airbyte connectors and write raw records to ClickHouse Bronze tables.

**Rationale**: Airbyte provides the extraction runtime, connection management, and protocol handling — eliminates the need for a custom connector runtime.

**Actors**: `cpt-insightspec-actor-ing-airbyte`, `cpt-insightspec-actor-ing-source-api`, `cpt-insightspec-actor-ing-clickhouse`

#### Nocode Connector Support

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-nocode-connector`

The system **MUST** support connectors defined as Airbyte declarative YAML manifests (nocode). Manifests **MUST** be stored in the connector package directory and registered with Airbyte via its API.

**Rationale**: Nocode connectors reduce development time from days to hours for standard REST/GraphQL APIs.

**Actors**: `cpt-insightspec-actor-ing-connector-author`, `cpt-insightspec-actor-ing-airbyte`

#### CDK Connector Support

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-cdk-connector`

The system **MUST** support connectors built with Airbyte's Python CDK for sources that require complex authentication, multi-step extraction, or custom transformation logic.

**Rationale**: Some sources cannot be handled by declarative manifests alone.

**Actors**: `cpt-insightspec-actor-ing-connector-author`, `cpt-insightspec-actor-ing-airbyte`

#### Tenant ID Injection

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-tenant-id`

Every connector **MUST** accept `tenant_id` as a configuration parameter and include it as a field in every emitted record. For nocode connectors, this **MUST** be implemented via the `AddFields` transformation in the declarative manifest. For CDK connectors, this **MUST** be implemented in the `parse_response()` method.

**Rationale**: Workspace-level data isolation is a platform invariant — all downstream layers (Silver, Gold) depend on `tenant_id` being present in every record.

**Actors**: `cpt-insightspec-actor-ing-connector-author`, `cpt-insightspec-actor-ing-workspace-admin`

#### Incremental Sync

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-incremental-sync`

Connectors **MUST** support incremental extraction using Airbyte's cursor-based sync. State **MUST** be persisted between runs to avoid full re-extraction.

**Rationale**: Full extraction is prohibitively slow and resource-intensive for large data sources.

**Actors**: `cpt-insightspec-actor-ing-airbyte`, `cpt-insightspec-actor-ing-source-api`

### 5.2 Orchestration

#### Pipeline Scheduling

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-kestra-scheduling`

Argo Workflows **MUST** schedule ingestion pipelines via CronWorkflows with configurable cron expressions. Each pipeline **MUST** define the complete extract-transform cycle for one or more connectors. Schedules are declared in connector `descriptor.yaml`.

**Rationale**: Automated scheduling is essential for continuous data freshness.

**Actors**: `cpt-insightspec-actor-ing-argo`, `cpt-insightspec-actor-ing-platform-engineer`

#### Task Dependency Management

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-kestra-dependency`

Argo Workflows **MUST** enforce task ordering via DAG templates: Airbyte sync **MUST** complete successfully before dbt transformation begins. Multiple independent syncs **MAY** run in parallel.

**Rationale**: dbt models read from Bronze tables — they must contain fresh data before transformation runs.

**Actors**: `cpt-insightspec-actor-ing-argo`

#### Retry on Failure

- [ ] `p2` - **ID**: `cpt-insightspec-fr-ing-kestra-retry`

Argo Workflows **SHOULD** retry failed tasks (Airbyte sync or dbt run) with configurable retry count and backoff strategy via `retryStrategy`.

**Rationale**: Transient API failures and network issues are common in data extraction.

**Actors**: `cpt-insightspec-actor-ing-argo`

### 5.3 Bronze Layer

#### Bronze Storage in ClickHouse

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-bronze-storage`

Raw extracted data **MUST** be stored in ClickHouse Bronze tables. Table names **MUST** match Airbyte stream names directly (no source prefix). Tables are created in a per-tenant database namespace (e.g., `bronze_{tenant_id}`). Tables **MUST** use `ReplacingMergeTree` engine with epoch millisecond versioning.

**Rationale**: Stream-name-based table naming enables automated discovery; ReplacingMergeTree provides idempotent upserts.

**Actors**: `cpt-insightspec-actor-ing-clickhouse`, `cpt-insightspec-actor-ing-airbyte`

#### Source-Native Schema Preservation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-bronze-schema-native`

Bronze tables **MUST** preserve the source-native schema and field names. No transformations **MUST** occur at the Bronze layer beyond what Airbyte's destination connector applies (type mapping, column naming).

**Rationale**: Bronze is the raw audit layer — transformations happen in dbt.

**Actors**: `cpt-insightspec-actor-ing-airbyte`, `cpt-insightspec-actor-ing-clickhouse`

#### ClickHouse as Primary Destination

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-clickhouse-destination`

Airbyte **MUST** use ClickHouse as the primary destination for all connector syncs. Shard-local table placement **MUST** be supported.

**Rationale**: ClickHouse is the platform's analytical storage engine.

**Actors**: `cpt-insightspec-actor-ing-airbyte`, `cpt-insightspec-actor-ing-clickhouse`

### 5.4 Silver Step 1 Transformations

#### dbt Bronze-to-Silver Transformations

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-dbt-bronze-to-silver`

dbt models **MUST** transform Bronze tables into Silver step 1 unified tables (`class_{domain}`). Each connector package **MUST** include dbt models (e.g., `dbt/to_git.sql`, `dbt/to_collaboration.sql`) for its Silver targets.

**Rationale**: Silver tables provide the unified schema that downstream analytics and identity resolution depend on.

**Actors**: `cpt-insightspec-actor-ing-dbt`, `cpt-insightspec-actor-ing-connector-author`

#### dbt-clickhouse Adapter

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-dbt-clickhouse`

dbt transformations **MUST** use the `dbt-clickhouse` adapter to execute against the ClickHouse cluster.

**Rationale**: All data resides in ClickHouse — the adapter must match the storage engine.

**Actors**: `cpt-insightspec-actor-ing-dbt`, `cpt-insightspec-actor-ing-clickhouse`

#### Unified Silver Schema

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-silver-unified-schema`

Silver tables **MUST** follow the established unified schemas (`class_commits`, `class_task_tracker_activities`, `class_comms_events`, `class_people`, etc.). dbt models **MUST** map source-specific fields to the unified schema. `tenant_id` **MUST** be preserved in all Silver tables.

**Rationale**: Cross-source analytics require a common schema.

**Actors**: `cpt-insightspec-actor-ing-dbt`, `cpt-insightspec-actor-ing-connector-author`

### 5.5 Infrastructure Management

#### Connection Management via API

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-terraform-connections`

Airbyte connections (source + destination + configured catalog) **MUST** be managed via `airbyte-toolkit/connect.sh` which calls the Airbyte API directly. Connection configurations **MUST** be defined in tenant YAML files (`connections/{tenant}.yaml`) and connector descriptors (`descriptor.yaml`). Credentials **MUST NOT** be committed to version control.

**Rationale**: Configuration as code prevents drift and enables reproducible deployments. Direct API calls avoid Terraform provider version compatibility issues.

**Actors**: `cpt-insightspec-actor-ing-toolbox`, `cpt-insightspec-actor-ing-platform-engineer`

#### Custom Connector Registration via API

- [ ] `p2` - **ID**: `cpt-insightspec-fr-ing-airbyte-api-custom`

Custom connectors (nocode manifests and CDK Docker images) **MUST** be registered with Airbyte via `airbyte-toolkit/register.sh` which calls the Airbyte API.

**Rationale**: Connector definitions are dynamic and tightly coupled to the connector package lifecycle.

**Actors**: `cpt-insightspec-actor-ing-airbyte`, `cpt-insightspec-actor-ing-platform-engineer`

### 5.6 Connector Packaging

#### Self-Contained Connector Packages

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-package-structure`

Each connector **MUST** be packaged as a directory at `/src/ingestion/connectors/{connector_class}/{source_name}/` containing: (1) connector definition (`connector.yaml` for nocode or `src/` for CDK code), (2) dbt models in `dbt/` (e.g., `dbt/to_git.sql`, `dbt/to_collaboration.sql`, `dbt/to_identity_resolution.sql`) for Bronze-to-Silver transformations, (3) a `descriptor.yaml` declaring the package name, Silver targets, and stream definitions.

**Rationale**: Self-contained packages enable independent development, testing, and deployment of connectors with their transformations.

**Actors**: `cpt-insightspec-actor-ing-connector-author`

#### Monorepo Package Storage

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-package-monorepo`

All connector packages **MUST** reside in the project monorepo. No external package registry is required.

**Rationale**: Simplifies dependency management and ensures all packages are version-controlled together.

**Actors**: `cpt-insightspec-actor-ing-connector-author`, `cpt-insightspec-actor-ing-platform-engineer`

### 5.7 Security

#### Secret Management

- [ ] `p1` - **ID**: `cpt-insightspec-fr-ing-secret-management`

Credentials (API keys, OAuth tokens, client secrets) **MUST** be stored in gitignored tenant YAML files (`connections/{tenant}.yaml`) or an external secret manager (Kubernetes Secrets, Vault). Credentials **MUST NOT** be stored in connector manifests or version control. Each connector provides a `credentials.yaml.example` template documenting required fields.

**Rationale**: Credential leakage is a critical security risk.

**Actors**: `cpt-insightspec-actor-ing-platform-engineer`, `cpt-insightspec-actor-ing-workspace-admin`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Idempotent Extraction

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-ing-idempotency`

Re-running an Airbyte sync for the same time range **MUST** produce the same result in Bronze tables. ClickHouse `ReplacingMergeTree` with version column **MUST** handle duplicate records.

**Threshold**: Zero duplicate records after sync re-run.

**Verification**: Run same sync twice; verify record count and content are identical after `OPTIMIZE TABLE FINAL`.

#### Error Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-ing-error-isolation`

A failure in one connector sync **MUST NOT** affect other connector syncs or dbt transformations for unrelated connectors.

**Threshold**: Zero cross-connector failure propagation.

**Verification**: Fail one connector intentionally; verify all other pipelines complete successfully.

#### Tenant Data Isolation

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-ing-tenant-isolation`

Data from one workspace **MUST NOT** be accessible or mixed with data from another workspace. `tenant_id` **MUST** be present and correct in every Bronze and Silver record.

**Threshold**: Zero cross-workspace data leakage.

**Verification**: Query Bronze and Silver tables; verify no records exist with incorrect or missing `tenant_id`.

#### Observability

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-ing-observability`

Pipeline execution **SHOULD** provide visibility into: sync status, record counts, execution duration, error details, and dbt model status. Argo Workflows UI and Airbyte UI **SHOULD** serve as primary observability surfaces.

**Threshold**: All pipeline executions visible in Argo UI within 1 minute.

**Verification**: Trigger a pipeline; verify execution appears in Argo UI with status, duration, and task details.

### 6.2 NFR Exclusions

- ClickHouse cluster performance tuning (infrastructure concern)
- Network latency optimization to source APIs (external dependency)
- Airbyte platform high-availability configuration (infrastructure concern)

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Argo Workflows API

- [ ] `p1` - **ID**: `cpt-insightspec-interface-ing-argo-api`

Argo Workflows API and CLI for submitting workflows, querying execution status, and managing CronWorkflow schedules.

#### Airbyte API

- [ ] `p1` - **ID**: `cpt-insightspec-interface-ing-airbyte-api`

Airbyte Public API for managing sources, destinations, connections, triggering syncs, and registering custom connectors.

### 7.2 External Integration Contracts

#### Airbyte Protocol

- [ ] `p1` - **ID**: `cpt-insightspec-contract-ing-airbyte-protocol`

Airbyte Protocol v2 defines the message format between connectors and destinations: RECORD, STATE, LOG, TRACE, CONTROL, SPEC, CATALOG, CONNECTION_STATUS message types.

#### dbt Contracts

- [ ] `p1` - **ID**: `cpt-insightspec-contract-ing-dbt-contracts`

dbt model contracts via `schema.yml`: column types, not-null constraints, accepted values, and relationships.

#### Airbyte REST API

- [ ] `p1` - **ID**: `cpt-insightspec-contract-ing-airbyte-rest-api`

Airbyte REST API (v1) for programmatic management of sources, destinations, connections, and connector definitions. Used by `airbyte-toolkit/connect.sh` and `airbyte-toolkit/register.sh`.

## 8. Use Cases

### Use Case 1: Create a New Nocode Connector Package

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-ing-new-nocode-connector`

**Actors**: `cpt-insightspec-actor-ing-connector-author`

**Preconditions**:

- Connector author has API documentation for the target source
- Unified Silver schema exists for the target domain

**Main Flow**:

1. Author creates a connector directory in the monorepo: `src/ingestion/connectors/{connector_class}/{source_name}/`
2. Author writes a declarative manifest (`connector.yaml`) defining streams, authentication, pagination, and incremental sync
3. Author adds `tenant_id` injection via `AddFields` transformation in the manifest
4. Author writes dbt models in `src/ingestion/connectors/{connector_class}/{source_name}/dbt/` (e.g., `to_git.sql`, `to_collaboration.sql`) mapping Bronze fields to Silver schema
5. Author creates `descriptor.yaml` declaring package name, Silver targets, and stream definitions
6. Author tests locally using ultra-light debugging (source.sh) or Kind K8s cluster
7. Author registers the connector with Airbyte via `update-connectors.sh`
8. Platform engineer creates Airbyte connection via `update-connections.sh`

**Postconditions**:

- Connector extracts data, writes to Bronze, dbt transforms to Silver

### Use Case 2: Scheduled Ingestion Pipeline Run

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-ing-scheduled-run`

**Actors**: `cpt-insightspec-actor-ing-argo`, `cpt-insightspec-actor-ing-airbyte`, `cpt-insightspec-actor-ing-dbt`

**Preconditions**:

- Connector is registered in Airbyte
- Connection is configured by `reconcile-connectors.sh`
- CronWorkflow is applied by `reconcile-connectors.sh` (also done in-cluster on a schedule)

**Main Flow**:

1. Argo CronWorkflow triggers the pipeline on schedule (cron)
2. Argo calls Airbyte API to start sync for the configured connection
3. Airbyte runs the connector, extracts data from source API with `tenant_id`
4. Airbyte writes records to ClickHouse Bronze tables
5. Argo detects sync completion, triggers `dbt run` for the connector's models
6. dbt transforms Bronze data to Silver step 1 tables
7. Argo marks the workflow as successful

**Alternative Flows**:

- **Airbyte sync failure**: Argo retries according to `retryStrategy`. If all retries fail, workflow is marked as failed.

**Postconditions**:

- Bronze and Silver tables contain fresh data with correct `tenant_id`

### Use Case 3: Local Connector Debugging

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-ing-local-debug`

**Actors**: `cpt-insightspec-actor-ing-connector-author`

**Preconditions**:

- Docker is installed
- Connector manifest is written

**Main Flow**:

1. Author places connector manifest and credentials in the connector directory
2. Author runs `source.sh check {connector}` to validate credentials
3. Author runs `source.sh discover {connector}` to list available streams
4. Author runs `source.sh read {connector} {connection}` to extract sample data
5. Author inspects JSON output for correctness (schema, `tenant_id` presence)
6. Optionally, author pipes output to a local destination for integration testing

**Postconditions**:

- Author has validated connector behavior without deploying to Airbyte

**Reference**: [Airbyte Connector DESIGN](../../connector/specs/DESIGN.md)

### Use Case 4: Add Data Source to Workspace

- [ ] `p1` - **ID**: `cpt-insightspec-usecase-ing-add-source-to-workspace`

**Actors**: `cpt-insightspec-actor-ing-workspace-admin`, `cpt-insightspec-actor-ing-platform-engineer`, `cpt-insightspec-actor-ing-toolbox`

**Preconditions**:

- Connector package exists
- Airbyte platform is running

**Main Flow**:

1. Workspace admin provides credentials in `connections/{tenant}.yaml`
2. Platform engineer runs `./reconcile-connectors.sh` — creates the source and connection in Airbyte (the shared bronze destination is resolved, created once, per ADR-0012) and applies the per-connector CronWorkflow in Argo (idempotent; the in-cluster reconcile CronWorkflow does the same on a schedule)
3. CronWorkflow picks up the new connection on next scheduled run

**Postconditions**:

- Data from the new source flows through the pipeline with the correct `tenant_id`

## 9. Acceptance Criteria

- A nocode connector package can be created, tested locally, registered with Airbyte, and produce data in Bronze and Silver tables
- A CDK connector package follows the same packaging and deployment workflow
- Argo Workflows successfully orchestrates the full extract-transform cycle (Airbyte sync -> dbt run)
- All Bronze and Silver records contain a valid `tenant_id`
- `update-connections.sh` can create and update Airbyte connections from tenant YAML configs
- Pipeline failures in one connector do not affect other connectors
- Incremental sync correctly resumes from the last cursor position
- Local debugging workflow (source.sh) works for nocode connectors without Airbyte platform

## 10. Dependencies


| Dependency                   | Type              | Description                                                                                       |
| ---------------------------- | ----------------- | ------------------------------------------------------------------------------------------------- |
| Airbyte                      | External platform | Connector execution, connection management, catalog discovery                                     |
| Argo Workflows               | External platform | Pipeline scheduling, DAG orchestration, retry, observability                                      |
| dbt-clickhouse               | External tool     | SQL transformation framework with ClickHouse adapter                                              |
| ClickHouse                   | External database | Bronze and Silver data storage                                                                    |
| Identity Resolution          | Internal domain   | Downstream consumer of Silver step 1 tables ([DESIGN](../../identity-resolution/specs/DESIGN.md)) |


## 11. Assumptions

- Airbyte's ClickHouse destination supports shard-local table writes and `ReplacingMergeTree` engine
- Argo Workflows is stable on Kind K8s for local development and managed K8s for production
- Declarative manifests can express all required authentication and pagination patterns for planned sources
- ClickHouse cluster is provisioned and accessible from the Kubernetes cluster

## 12. Risks


| Risk                                                 | Impact                                                 | Mitigation                                                               |
| ---------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------ |
| Airbyte ClickHouse destination limitations           | Cannot write to shard-local tables correctly           | Evaluate destination capabilities early; fork destination if needed      |
| Argo Workflows version incompatibility               | Pipeline orchestration unreliable                      | Pin Argo version in Helm values; test upgrades in local Kind first      |
| Declarative manifest limitations for complex sources | Must fall back to CDK for more connectors than planned | CDK path is fully supported; declarative is preferred, not exclusive     |
| `tenant_id` enforcement gaps                         | Data without `tenant_id` reaches Silver                | Automated validation in dbt tests (`not_null` on `tenant_id`)           |
| Connection config drift                              | Airbyte connections out of sync with YAML configs      | Re-run `update-connections.sh` on deploy; state persisted in K8s ConfigMaps |

