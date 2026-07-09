---
status: proposed
date: 2026-03-23
---

# DESIGN -- Ingestion Layer

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
  - [3.8 Connector Package Structure](#38-connector-package-structure)
- [4. Deployment](#4-deployment)
  - [4.1 Production (Kubernetes + Helm)](#41-production-kubernetes--helm)
  - [4.2 Local Development (Kind K8s Cluster)](#42-local-development-kind-k8s-cluster)
  - [4.3 Ultra-Light Connector Debugging](#43-ultra-light-connector-debugging)
  - [4.4 Schema Migrations](#44-schema-migrations)
- [5. Additional Context](#5-additional-context)
  - [5.1 Superseded Components](#51-superseded-components)
- [6. Open Questions](#6-open-questions)
  - [OQ-ING-01: ClickHouse Destination Normalization Mode](#oq-ing-01-clickhouse-destination-normalization-mode)
  - [OQ-ING-02: ~~Kestra Flow Granularity~~ (RESOLVED)](#oq-ing-02-kestra-flow-granularity-resolved)
  - [OQ-ING-03: MariaDB Destination Use Cases](#oq-ing-03-mariadb-destination-use-cases)
  - [OQ-ING-04: Gold Layer Ownership](#oq-ing-04-gold-layer-ownership)
  - [OQ-ING-05: Connector Package Versioning](#oq-ing-05-connector-package-versioning)
- [7. Traceability](#7-traceability)

<!-- /toc -->

---

## 1. Architecture Overview

### 1.1 Architectural Vision

The Ingestion Layer provides the complete data pipeline from external source APIs to unified Silver step 1 tables, built on industry-standard open-source tools. Airbyte handles data extraction through both nocode declarative manifests and Python CDK connectors. Argo Workflows orchestrates the pipeline lifecycle -- scheduling syncs via CronWorkflows, managing dependencies between extraction and transformation via DAG templates, and handling retries with configurable retry strategies. dbt-clickhouse transforms raw Bronze data into unified Silver schemas. Terraform manages Airbyte connection configuration as code.

This layer replaces the previously designed custom Orchestrator and custom Connector Framework with a simpler, more maintainable stack that requires no custom runtime code.

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|---|---|
| `cpt-insightspec-fr-ing-airbyte-extract` | Airbyte Platform with ClickHouse destination handles extraction and loading |
| `cpt-insightspec-fr-ing-nocode-connector` | Airbyte declarative manifests (YAML) with `source-declarative-manifest` base image |
| `cpt-insightspec-fr-ing-cdk-connector` | Airbyte Python CDK with custom Docker images |
| `cpt-insightspec-fr-ing-tenant-id` | `AddFields` transformation in manifests; explicit injection in CDK `read_records()` |
| `cpt-insightspec-fr-ing-incremental-sync` | Airbyte cursor-based incremental sync with persisted state |
| `cpt-insightspec-fr-ing-kestra-scheduling` | Argo CronWorkflows with cron triggers for scheduled execution |
| `cpt-insightspec-fr-ing-kestra-dependency` | Argo DAG templates with explicit task dependencies: sync -> dbt |
| `cpt-insightspec-fr-ing-bronze-storage` | ClickHouse `ReplacingMergeTree` with Airbyte stream name as table name |
| `cpt-insightspec-fr-ing-dbt-bronze-to-silver` | Per-package dbt models producing `class_{domain}` tables |
| `cpt-insightspec-fr-ing-terraform-connections` | Airbyte Terraform provider for connection management |
| `cpt-insightspec-fr-ing-package-structure` | Self-contained connector packages: manifest + dbt + descriptor |

#### NFR Allocation

| NFR | Component | Verification |
|-----|-----------|-------------|
| `cpt-insightspec-nfr-ing-idempotency` | ClickHouse ReplacingMergeTree | Re-run sync; verify no duplicates after OPTIMIZE TABLE FINAL |
| `cpt-insightspec-nfr-ing-error-isolation` | Argo per-task execution in DAG | Fail one connector; verify others succeed |
| `cpt-insightspec-nfr-ing-tenant-isolation` | Connector-level tenant_id injection | Query tables; verify no missing/incorrect tenant_id |
| `cpt-insightspec-nfr-ing-observability` | Argo UI + Airbyte UI | Verify executions visible within 1 minute |

#### Architecture Decision Records

| ADR | Decision |
|-----|----------|
| [ADR-0001](ADR/0001-kestra-over-airflow.md) `cpt-insightspec-adr-ing-kestra-over-airflow` | Use Kestra over Airflow — YAML-first, no Python dependency (superseded) |
| [ADR-0002](ADR/0002-argo-over-kestra.md) `cpt-insightspec-adr-argo-over-kestra` | Use Argo Workflows over Kestra — K8s-native, no external DB dependency |
| [ADR-0003](ADR/0003-k8s-secrets-credentials.md) `cpt-insightspec-adr-k8s-secrets-credentials` | Use K8s Secrets as primary credential source — vendor-neutral, dev/prod parity |

### 1.3 Architecture Layers

```
┌──────────────────────────────────────────────────────────────────┐
│                      EXTERNAL DATA SOURCES                       │
│  GitHub · Jira · MS365 · GitLab · Slack · BambooHR · etc.       │
└───────────────────────┬──────────────────────────────────────────┘
                        │ HTTP / REST / GraphQL
                        ▼
┌──────────────────────────────────────────────────────────────────┐
│                     AIRBYTE PLATFORM                             │
│                                                                  │
│  ┌─────────────────┐  ┌─────────────────┐  ┌────────────────┐  │
│  │ Nocode Connectors│  │ CDK Connectors  │  │ ClickHouse     │  │
│  │ (YAML manifests) │  │ (Python)        │  │ Destination    │  │
│  └────────┬────────┘  └────────┬────────┘  └───────┬────────┘  │
│           │  tenant_id         │  tenant_id         │           │
│           └────────────────────┴────────────────────┘           │
│                                                                  │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │ Airbyte API  · Connection Management · Catalog Discovery │    │
│  └─────────────────────────────────────────────────────────┘    │
└───────────────────────┬──────────────────────────────────────────┘
                        │ Airbyte Protocol (RECORD, STATE)
                        ▼
┌──────────────────────────────────────────────────────────────────┐
│                    BRONZE LAYER (ClickHouse)                     │
│                                                                  │
│  Airbyte stream name tables · ReplacingMergeTree · shard-local  │
│  tenant_id in every record · source-native schema preserved     │
└───────────────────────┬──────────────────────────────────────────┘
                        │ SQL (dbt-clickhouse)
                        ▼
┌──────────────────────────────────────────────────────────────────┐
│               SILVER STEP 1 LAYER (ClickHouse)                   │
│                                                                  │
│  class_{domain} tables · unified schema · tenant_id             │
│  source-native user IDs intact (person_id added in step 2)      │
└──────────────────────────────────────────────────────────────────┘

                  ORCHESTRATION (ARGO WORKFLOWS)
        ┌──────────────────────────────────────┐
        │  CronWorkflow → Airbyte Sync → dbt   │
        │  DAG · Retry · Observability          │
        └──────────────────────────────────────┘

                INFRASTRUCTURE AS CODE (TERRAFORM)
        ┌──────────────────────────────────────┐
        │  Airbyte Connections · CI/CD Apply    │
        └──────────────────────────────────────┘
```

## 2. Principles & Constraints

### 2.1 Design Principles

#### Self-Contained Connector Packages

- [ ] `p1` - **ID**: `cpt-insightspec-principle-ing-package-self-contained`

Each connector package contains everything needed for its pipeline: connector definition (manifest or code), dbt models for Bronze-to-Silver transformation, and a descriptor YAML declaring its capabilities. No external dependencies beyond the package directory.

**Why**: Enables independent development, testing, and deployment of connectors.

#### No Custom Runtime

- [ ] `p1` - **ID**: `cpt-insightspec-principle-ing-no-custom-runtime`

Use Airbyte's connector runtime for extraction and Argo Workflows' execution engine for orchestration. No custom runner, subprocess management, or message protocol.

**Why**: Reduces maintenance burden; leverages battle-tested runtimes with community support.

#### Declarative-First

- [ ] `p1` - **ID**: `cpt-insightspec-principle-ing-declarative-first`

Prefer nocode declarative manifests for new connectors. Use CDK (Python) only when the declarative approach cannot express the required logic (complex auth, multi-step API calls, binary data).

**Why**: Declarative manifests are faster to develop, easier to review, and simpler to maintain.

#### Tenant Isolation at Source

- [ ] `p1` - **ID**: `cpt-insightspec-principle-ing-tenant-isolation`

Every record is tagged with `tenant_id` at the connector level -- before data leaves the extraction boundary. This is not a post-processing step; it is an extraction invariant.

**Why**: Guarantees tenant isolation from the earliest point in the pipeline.

#### Silver Targets Known at Design Time

- [ ] `p1` - **ID**: `cpt-insightspec-principle-ing-silver-at-design-time`

When creating a connector package, the author knows which Silver tables (`class_{domain}`) the connector will populate. The descriptor YAML declares these targets explicitly.

**Why**: Enables package-level validation and dependency tracking.

#### Airbyte Resource Identity by ID

All automation scripts identify Airbyte resources (source definitions, sources, destinations, connections, builder projects) exclusively by their Airbyte-assigned UUID stored in the state file. Name-based matching is prohibited because multiple resources can share the same name, and names are not stable across Airbyte reinstalls. If a stored ID is no longer valid (Airbyte returns 404), the resource is recreated and the new ID is saved.

**Why**: Prevents stale reference errors after Airbyte reinstalls and avoids ambiguity when multiple resources share a name.

### 2.2 Constraints

#### ClickHouse as Primary Destination

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-ing-clickhouse-destination`

All Bronze and Silver data resides in ClickHouse. MariaDB is an alternative destination for specific use cases (documented separately). No other storage engines are supported.

#### Monorepo Package Storage

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-ing-monorepo`

All connector packages reside in the project monorepo. No external package registry, no Git submodules, no npm/pip packages.

#### No External Connector Registry

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-ing-no-external-registry`

Custom connectors are registered directly with the Airbyte instance via its API. There is no separate registry service.

#### Terraform for Connections Only

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-ing-terraform-connections`

Terraform manages Airbyte connections (source + destination + catalog). Custom connector registration is handled via Airbyte API, not Terraform.

## 3. Technical Architecture

### 3.1 Domain Model

```mermaid
classDiagram
    class ConnectorPackage {
        +String name
        +String class
        +String source
        +String type [nocode|cdk]
        +ConnectorDefinition definition
        +DbtModels dbt
        +Descriptor descriptor
    }
    class ConnectorDefinition {
        <<abstract>>
    }
    class DeclarativeManifest {
        +String version
        +Stream[] streams
        +Spec spec
    }
    class CdkConnector {
        +Dockerfile dockerfile
        +PythonSource source
    }
    class Descriptor {
        +String name
        +String version
        +String type
        +String[] silver_targets
        +StreamDef[] streams
    }
    class StreamDef {
        +String name
        +String bronze_table
        +String[] primary_key
        +String cursor_field
    }
    class DbtModels {
        +SqlFile[] models
        +SchemaYml schema
    }
    class AirbyteConnection {
        +String source_id
        +String destination_id
        +ConfiguredCatalog catalog
        +String tenant_id
    }
    class ArgoWorkflow {
        +String name
        +String namespace
        +String schedule
        +DAGTemplate dag
    }
    class BronzeTable {
        +String name [stream_name]
        +String engine [ReplacingMergeTree]
        +Column tenant_id
    }
    class SilverTable {
        +String name [class_domain]
        +Column tenant_id
    }

    ConnectorPackage *-- ConnectorDefinition
    ConnectorPackage *-- DbtModels
    ConnectorPackage *-- Descriptor
    ConnectorDefinition <|-- DeclarativeManifest
    ConnectorDefinition <|-- CdkConnector
    Descriptor *-- StreamDef
    AirbyteConnection --> ConnectorPackage : uses
    ArgoWorkflow --> AirbyteConnection : triggers sync
    ArgoWorkflow --> DbtModels : triggers run
    AirbyteConnection --> BronzeTable : writes to
    DbtModels --> BronzeTable : reads from
    DbtModels --> SilverTable : writes to
```

### 3.2 Component Model

#### Airbyte Platform

- [ ] `p1` - **ID**: `cpt-insightspec-component-ing-airbyte`

##### Why this component exists

Handles all data extraction from external sources, providing connector runtime, connection management, catalog discovery, and data delivery to ClickHouse.

##### Responsibility scope

- Execute nocode (declarative) and CDK (Python) connectors
- Manage source and destination configurations
- Discover source schemas (catalog)
- Deliver extracted records to ClickHouse via Airbyte Protocol
- Persist incremental sync state between runs
- Provide API for connection management and sync triggering

##### Responsibility boundaries

- Does NOT perform data transformation (that is dbt's responsibility)
- Does NOT schedule or orchestrate pipelines (that is Argo Workflows' responsibility)
- Does NOT manage its own connection configurations in version control (that is Terraform's responsibility)

##### Related components (by ID)

- `cpt-insightspec-component-ing-argo` -- Argo Workflows triggers Airbyte syncs via HTTP
- `cpt-insightspec-component-ing-terraform` -- Terraform manages Airbyte connections
- `cpt-insightspec-component-ing-clickhouse` -- ClickHouse is the sync destination

#### Argo Workflows Orchestrator

- [ ] `p1` - **ID**: `cpt-insightspec-component-ing-argo`

##### Why this component exists

Provides pipeline scheduling, task dependency management, retry handling, and execution observability. Selected over Kestra to eliminate PostgreSQL dependency — Argo stores state in K8s etcd (see [ADR-0002](ADR/0002-argo-over-kestra.md)).

##### Responsibility scope

- Schedule pipeline CronWorkflows with cron expressions
- Trigger Airbyte syncs via HTTP POST to Airbyte API
- Poll for sync completion and check status
- Trigger dbt runs via container steps (`insight-toolbox`)
- Enforce task ordering via DAG templates (sync before transform)
- Retry failed tasks with configurable backoff (`retryStrategy`)
- Provide Argo UI for monitoring and manual execution

##### Responsibility boundaries

- Does NOT execute extraction logic (Airbyte does)
- Does NOT execute transformation SQL (dbt does)
- Does NOT manage connection configurations (Terraform does)

##### Related components (by ID)

- `cpt-insightspec-component-ing-airbyte` -- triggers syncs, monitors completion
- `cpt-insightspec-component-ing-dbt` -- triggers dbt runs after sync

#### dbt-clickhouse

- [ ] `p1` - **ID**: `cpt-insightspec-component-ing-dbt`

##### Why this component exists

Transforms raw Bronze data into unified Silver step 1 tables using SQL. Each connector package includes its own dbt models.

##### Responsibility scope

- Execute Bronze-to-Silver SQL transformations
- Map source-specific fields to unified Silver schemas
- Preserve `tenant_id` in all Silver tables
- Validate data quality via dbt tests (not_null, unique, accepted_values)
- Document column definitions in schema.yml

##### Responsibility boundaries

- Does NOT extract data from external sources (Airbyte does)
- Does NOT schedule its own execution (Argo Workflows does)
- Does NOT perform identity resolution (separate domain)

##### Related components (by ID)

- `cpt-insightspec-component-ing-argo` -- Argo Workflows triggers dbt runs
- `cpt-insightspec-component-ing-clickhouse` -- dbt reads from and writes to ClickHouse

#### ClickHouse Cluster

- [ ] `p1` - **ID**: `cpt-insightspec-component-ing-clickhouse`

##### Why this component exists

Analytical storage engine for all Bronze and Silver data. Provides columnar storage, shard-local tables, and `ReplacingMergeTree` for idempotent upserts.

##### Responsibility scope

- Store Bronze tables (Airbyte stream names) with source-native schema
- Store Silver tables (`class_{domain}`) with unified schema
- Provide shard-local table placement for distributed queries
- Handle record deduplication via `ReplacingMergeTree` versioning

##### Responsibility boundaries

- Does NOT manage its own schema migrations (Airbyte destination creates Bronze; dbt creates Silver)
- Does NOT provide application-level access control (handled at platform level)

##### Related components (by ID)

- `cpt-insightspec-component-ing-airbyte` -- Airbyte writes Bronze data
- `cpt-insightspec-component-ing-dbt` -- dbt reads Bronze, writes Silver

#### Terraform

- [ ] `p1` - **ID**: `cpt-insightspec-component-ing-terraform`

##### Why this component exists

Manages Airbyte connection configurations as code, enabling version control, PR review, and CI/CD deployment of pipeline configurations.

##### Responsibility scope

- Define Airbyte sources, destinations, and connections in HCL
- Apply configurations via `terraform plan` / `terraform apply`
- Manage state in remote backend (S3/GCS)
- Run in CI/CD pipelines

##### Responsibility boundaries

- Does NOT register custom connectors (Airbyte API handles registration)
- Does NOT manage Argo WorkflowTemplate definitions (stored as YAML in Git)
- Does NOT manage ClickHouse cluster infrastructure

##### Related components (by ID)

- `cpt-insightspec-component-ing-airbyte` -- manages Airbyte connections via API

### 3.3 API Contracts

#### Airbyte Protocol

Airbyte Protocol v2 defines structured JSON messages between connectors and the platform:
- `RECORD` -- data record with stream name and fields (including `tenant_id`)
- `STATE` -- incremental sync cursor for resumable extraction
- `LOG` -- connector execution logs
- `TRACE` -- detailed execution tracing
- `CATALOG` -- stream schema discovery results
- `SPEC` -- connector specification (configuration schema)

#### Argo WorkflowTemplate / CronWorkflow YAML

Argo Workflows uses Kubernetes CRDs for workflow definitions:
```yaml
apiVersion: argoproj.io/v1alpha1
kind: CronWorkflow
metadata:
  name: m365-sync
  namespace: argo
spec:
  schedule: "0 2 * * *"
  timezone: UTC
  concurrencyPolicy: Replace
  workflowSpec:
    entrypoint: pipeline
    arguments:
      parameters:
        - name: connection_id
          value: "CONNECTION_ID"
    templates:
      - name: pipeline
        dag:
          tasks:
            - name: sync
              templateRef:
                name: airbyte-sync
                template: sync
            - name: transform
              depends: sync
              templateRef:
                name: dbt-run
                template: run
```

#### Descriptor YAML Schema

Each connector package includes `descriptor.yaml`:
```yaml
name: ms365
version: "1.0"
type: nocode
silver_targets:
  - class_comms_events
  - class_people
streams:
  - name: email_activity
    bronze_table: email_activity
    primary_key: [unique]
    cursor_field: reportRefreshDate
  - name: teams_activity
    bronze_table: teams_activity
    primary_key: [unique]
    cursor_field: reportRefreshDate
```

#### Terraform Configuration

Airbyte connections managed via Terraform:
```hcl
resource "airbyte_connection" "ms365_to_clickhouse" {
  name           = "ms365-tenant-acme"
  source_id      = airbyte_source.ms365.id
  destination_id = airbyte_destination.clickhouse.id

  configurations = {
    namespace_definition = "custom_format"
    namespace_format     = "bronze_ms365"
    streams = [
      { name = "email_activity", sync_mode = "incremental" },
      { name = "teams_activity", sync_mode = "incremental" }
    ]
  }
}
```

### 3.4 Internal Dependencies

| From | To | Mechanism |
|------|----|-----------|
| Argo Workflows | Airbyte | HTTP POST (trigger sync, poll job status) |
| Argo Workflows | dbt | Container step (dbt-clickhouse image) |
| Airbyte | ClickHouse | Airbyte ClickHouse destination (JDBC) |
| dbt | ClickHouse | dbt-clickhouse adapter (native protocol) |
| Terraform | Airbyte | Airbyte Terraform provider (REST API) |

### 3.5 External Dependencies

| Dependency | Version Constraint | Purpose |
|-----------|-------------------|---------|
| Airbyte Platform | Self-managed, latest stable | Connector runtime and connection management |
| Argo Workflows | v3.6.x, Apache 2.0 (CNCF) | Pipeline orchestration (K8s-native, state in etcd) |
| dbt-clickhouse | Latest stable adapter | SQL transformations on ClickHouse |
| Terraform + airbytehq/airbyte provider | Provider >= 1.0 | Connection configuration as code |
| ClickHouse | Cluster deployment | Analytical storage |

### 3.6 Interactions & Sequences

#### Scheduled Pipeline Run

**ID**: `cpt-insightspec-seq-ing-scheduled-pipeline`

```mermaid
sequenceDiagram
    participant Argo as Argo Workflows
    participant A as Airbyte
    participant S as Source API
    participant CH as ClickHouse
    participant D as dbt (container)

    Argo->>A: POST /api/v1/jobs (trigger sync)
    A->>S: HTTP requests (with tenant_id in config)
    S-->>A: API responses (records)
    A->>A: Add tenant_id to each record
    A->>CH: Write RECORD messages to Bronze tables
    Argo->>A: GET /api/v1/jobs/{id} (poll status)
    A-->>Argo: Sync complete (status: succeeded)
    Argo->>D: Run dbt container (dbt run --select tag:silver)
    D->>CH: SELECT from Bronze tables
    D->>CH: INSERT INTO Silver class_{domain} tables
    D-->>Argo: Container exit 0
    Argo->>Argo: Mark workflow succeeded
```

#### Terraform Connection Management

**ID**: `cpt-insightspec-seq-ing-terraform-apply`

```mermaid
sequenceDiagram
    participant E as Engineer
    participant G as Git / CI
    participant T as Terraform
    participant A as Airbyte API

    E->>G: Push HCL changes (PR)
    G->>T: terraform plan
    T->>A: GET current state
    T-->>G: Plan output (diff)
    E->>G: Approve & merge
    G->>T: terraform apply
    T->>A: POST/PATCH sources, destinations, connections
    A-->>T: Resources created/updated
    T-->>G: Apply complete
```

### 3.7 Database schemas & tables

#### Bronze Tables

Naming: Tables match Airbyte stream names directly (e.g., `commits`, `email_activity`). Tables are created in a namespace/database specified in the Airbyte connection configuration (managed via Terraform), such as `bronze_github` or `bronze_ms365`.

Engine: `ReplacingMergeTree(_version)` where `_version` is epoch milliseconds.

Required columns in every Bronze table:

| Column | Type | Description |
|--------|------|-------------|
| `tenant_id` | String | Tenant isolation -- injected by connector |
| `_airbyte_raw_id` | String | Airbyte deduplication key |
| `_airbyte_extracted_at` | DateTime64 | Extraction timestamp |
| `_version` | UInt64 | ReplacingMergeTree version (epoch ms) |

Shard-local tables: All Bronze tables use shard-local placement for distributed query execution.

#### Silver Step 1 Tables

Naming: `class_{domain}` (e.g., `class_commits`, `class_comms_events`, `class_people`)

Produced by dbt models. Unified schema across sources. `tenant_id` preserved from Bronze.

### 3.8 Connector Package Structure

```
src/ingestion/connectors/
  {class}/{source}/
    connector.yaml            # Nocode: Airbyte declarative manifest
    # OR for CDK:
    # src/
    #   source_{source}/
    #     __init__.py
    #     source.py
    #     schemas/
    #   setup.py
    #   Dockerfile
    descriptor.yaml            # Package metadata
    dbt/
      to_{domain}.sql          # dbt model(s) producing class_{domain} Silver tables
      schema.yml               # Column docs + tests
```

See [Airbyte Connector DESIGN](../../connector/specs/DESIGN.md) for detailed connector development guide.

## 4. Deployment

### 4.1 Production (Kubernetes + Helm)

PR #224 unified the deployment model: Airbyte, Argo Workflows and the Insight umbrella are three separate Helm releases that all install into the **same namespace** (default `insight`). Multi-tenant deployments use multiple namespaces — each is self-contained.

```text
K8s Cluster
└── namespace: insight (release namespace — shared by all three Helm releases)
    ├── Helm release: airbyte (chart: airbyte/airbyte)
    │   ├── Airbyte Server (API + UI)
    │   ├── Airbyte Workers (connector execution)
    │   ├── Temporal (internal orchestration)
    │   └── Airbyte-bundled PostgreSQL (Airbyte metadata)
    ├── Helm release: argo-workflows (chart: argo/argo-workflows)
    │   ├── Argo Server (API + UI)
    │   └── Argo Controller (workflow execution)
    └── Helm release: insight (chart: charts/insight, this repo)
        ├── ClickHouse (StatefulSet — separate L2 release, see §4 below)
        ├── MariaDB (Bitnami subchart — per-service databases:
        │              analytics owns `insight`, identity-resolution
        │              owns `identity` per ADR-0006)
        ├── Redis (Bitnami subchart — cache for API Gateway / Analytics API)
        ├── Redpanda (subchart — Kafka-compatible event broker)
        ├── Analytics API (Rust/Axum)
        ├── Identity Resolution (Rust persons-store — see ADR-0006)
        ├── API Gateway (Rust/Axum, OIDC, proxy)
        └── Frontend (SPA, nginx)
```

Key deployment decisions:
- **Single-namespace model**: Airbyte, Argo and the Insight umbrella share the release namespace; cross-release service references resolve via plain DNS (e.g. `airbyte-airbyte-server-svc.insight.svc.cluster.local:8001`). Lifecycle is still independent — each release upgrades on its own schedule.
- Argo Workflows stores state in K8s etcd — no external database required.
- Airbyte ships its own bundled PostgreSQL for connector metadata (managed by the `airbyte/airbyte` chart, not by the umbrella).
- Helm charts: Airbyte via `airbyte/airbyte`, Argo via `argo/argo-workflows`, the Insight platform via `charts/insight` in this repo. On Cyberfabric-operated and local clusters all three are installed by the gitops Makefile (`cd deploy/gitops && make deploy ENV=<env>` — Airbyte/Argo as L2 system releases, the umbrella as the L3 app release); external consumers install them with their own tooling.
- ClickHouse, MariaDB, Redis and Redpanda are deployed as separate L2 releases in `insight-infra` (gitops `make system-*`); the umbrella no longer bundles them as subcharts (removed in chart `0.2.0`). The umbrella consumes them through the unified `<dep>.host / .port / .database / .username / .passwordSecret` shape — see `charts/insight/values.yaml`.
- ClickHouse runs as a separate L2 release in `insight-infra` (gitops `make system-clickhouse`); the umbrella addresses it through `clickhouse.host`/`.port` (default `clickhouse.insight-infra.svc.cluster.local:8123`) over the HTTP interface. Health probes use HTTP GET `/ping` (not `clickhouse-client` exec — avoids CLI flag parsing issues with auto-generated passwords).
- MariaDB per-service databases are provisioned by a Helm Hook Job (`charts/insight/templates/mariadb-init-svcdbs-job.yaml`, `pre-install,pre-upgrade`) that dials the external `mariadb.host` and runs `CREATE DATABASE` + `GRANT` for each owned database — idempotent (`IF NOT EXISTS`) across helm upgrades. Each owning service then runs its own SeaORM migrations at startup — see §4.4.2 and [ADR-0006](ADR/0006-service-owned-migrations.md).
- CDK connector build script (`airbyte-toolkit/build-connector.sh`) uses `CLUSTER_NAME` env var (default `insight`) for Kind image loading — not hardcoded.
- Airbyte port-forward uses `nohup ... & disown` to avoid blocking the terminal.
- Argo `dbt-run` WorkflowTemplate uses locally-built `insight-toolbox:local` image (with `imagePullPolicy: IfNotPresent`) — not `ghcr.io/constructorfabric/insight-toolbox:latest`. Local builds via `tools/toolbox/build.sh` pick up dbt model changes without requiring a registry push. Template also accepts `full_refresh` parameter (pass `--full-refresh` to recreate tables from scratch).
- CoreDNS is patched to use public DNS upstream (`8.8.8.8`, `8.8.4.4`) — WSL's `/etc/resolv.conf` points to an internal WSL nameserver that cannot reliably resolve external domains (e.g. `login.microsoftonline.com`). Patch lives in `scripts/dev/patch-coredns-wsl.sh` (idempotent, opt-out via `SKIP_COREDNS_PATCH=1`); Windows/WSL+Kind operators run it manually against their cluster after bootstrap.
- Gold views migration (`20260422000000_gold-views.sql`) references bronze tables from optional connectors (jira, m365, zoom). When the corresponding bronze table does not yet exist (no real connector data ingested yet), `scripts/create-bronze-placeholders.sh` creates empty placeholder tables with a minimal compatible schema so the gold-views migration succeeds on a partial install.
  - **Placeholder handoff caveat**: Airbyte ClickHouse destination v2.0.8+ throws an error on the first sync if the target bronze table exists with a schema that does not match the destination's expected schema for that stream. The placeholder schemas in `create-bronze-placeholders.sh` are intentionally minimal (only the columns referenced by gold views) — they are **not** a drop-in replacement for a native Airbyte-generated table. Before enabling a previously-placeholdered connector, the operator should manually `DROP TABLE` the placeholder(s) in ClickHouse so Airbyte can create them fresh on its first sync.
  - Runs automatically inside the `clickhouse-migrate` Helm Hook Job, immediately before the gold-view migrations (see §4.4.1).
- All credentials managed via Kubernetes Secrets (see §4.1.1).
- Service access via Ingress (PR #224): the umbrella chart configures `ingress-nginx` routes for Frontend, API Gateway, Airbyte UI and Argo UI; on a local Kubernetes cluster the Kind config maps host ports 80/443 to the ingress controller. Direct port-forwards (`kubectl port-forward`) remain available for debugging.

#### 4.1.1 Infrastructure Secrets

All infrastructure and connector credentials are stored in Kubernetes Secrets. No credentials are hardcoded in production manifests.

| Secret | Namespace | Purpose | Required Keys | Created by |
|--------|-----------|---------|---------------|------------|
| `insight-db-creds` | `insight` | Auto-generated infra credentials (ClickHouse, MariaDB, Redis) | `clickhouse-password`, `mariadb-password`, `mariadb-root-password`, `redis-password` | umbrella chart `secrets.yaml` (`lookup`-based, idempotent across `helm upgrade`) |
| `airbyte-auth-secrets` | `insight` | Airbyte internal auth | `instance-admin-password`, `instance-admin-client-id`, `instance-admin-client-secret`, `jwt-signature-secret` | Helm chart (auto) |
| `insight-mariadb` | `insight` | MariaDB root + app credentials (Bitnami subchart's own Secret — read-only mirror of the relevant keys from `insight-db-creds`) | `mariadb-root-password`, `mariadb-password` | Bitnami MariaDB subchart (auto) |
| `insight-{connector}-{source-id}` | `insight` | Per-connector credentials (see [ADR-0003](ADR/0003-k8s-secrets-credentials.md)) | Connector-specific (e.g. `azure_client_id`, `azure_client_secret`) | `secrets/apply.sh` |

All Insight components share the release namespace (default `insight`); override with `INSIGHT_NAMESPACE` for non-default installs. Argo UI uses `--auth-mode=client` in production — authentication via K8s ServiceAccount Bearer tokens, no Secret required.

**Resolution order** (all scripts):
1. Read from K8s Secret — sole credential source for all environments
2. If Secret missing → skip connector with error (no inline fallback)
3. ClickHouse password: the `clickhouse-migrate` Hook Job and ingestion workflows read `clickhouse-password` from `insight-db-creds` and authenticate to the external ClickHouse over its HTTP interface (ClickHouse `X-ClickHouse-User` / `X-ClickHouse-Key` headers — the password is never placed on the command line)

**Connector credentials** use label-based discovery: `app.kubernetes.io/part-of: insight` label + `insight.cyberfabric.com/connector` annotation. See [ADR-0003](ADR/0003-k8s-secrets-credentials.md) for details.

#### Credential Resolution Rules

1. **K8s Secrets are the sole credential source.** Connector parameters — API keys, OAuth secrets, start dates, any configuration required by the connector spec — are stored exclusively in K8s Secrets. Tenant YAML files contain only `tenant_id`.

2. **Secret annotations define connector binding.** Each Secret carries:
   - Label `app.kubernetes.io/part-of: insight` — for discovery
   - Annotation `insight.cyberfabric.com/connector` — connector name (must match `descriptor.yaml` `name` field)
   - Annotation `insight.cyberfabric.com/source-id` — unique instance identifier

3. **No inline credential fallback.** Scripts do not fall back to reading credentials from tenant YAML. If a Secret is missing, the connector is skipped with an error.

4. **Destination password sync.** The reconcile loop (`reconcile-connectors/main.sh`) resolves the Airbyte bronze destination from `insight-db-creds` on every run (ADR-0012), so a rotated ClickHouse password takes effect without recreating connections.

5. **Password rotation procedure.** Update `insight-db-creds` → apply to cluster → rotate the credential on the external ClickHouse L2 release in `insight-infra` (owned by that release). The reconcile loop (`reconcile-connectors/main.sh`, run by the reconcile CronWorkflow) then re-syncs the Airbyte bronze-destination password from `insight-db-creds` on its next tick (ADR-0012).

6. **Destination sync modes.** The reconcile loop assigns destination sync modes based on source stream capabilities:
   - `full_refresh` streams → `overwrite` (each sync replaces all data — no duplicate accumulation)
   - `incremental` streams → `append_dedup` (appends new records, deduplicates by primary key)

### 4.2 Local Development (local Kubernetes cluster)

Ingestion work needs Airbyte and Argo Workflows, which the docker-compose dev
stack does not ship — so the ingestion inner loop runs against a local
Kubernetes cluster (Kind / OrbStack / k3d / minikube) brought up by the gitops
path. (For backend / frontend work that does not touch ingestion, the
docker-compose stack — `./dev-compose.sh up` — is the lighter default; see
[CONTRIBUTING.md](../../../../CONTRIBUTING.md).)

The gitops Makefile with `ENV=local` brings up the whole stack:
- Airbyte installed via Helm chart (`airbyte/airbyte`) as a separate Helm release
- Argo Workflows installed via Helm chart (`argo/argo-workflows`) as a separate Helm release
- Insight platform installed via the umbrella chart `charts/insight` — the app services (Analytics API, Identity Resolution, API Gateway, Frontend) plus, depending on the `<svc>.deploy` toggles, the bundled ClickHouse / MariaDB / Redis / Redpanda subcharts or wiring to externally-provided instances.
- dbt runs as Argo container steps (`insight-toolbox`)

`ENV=local` runs the wizard once and then the `bootstrap → system → deploy-app`
chain. The cluster topology, layer model and namespace conventions are owned by
the [gitops SPEC](../../components/deployment/gitops/README.md).

#### Startup scripts

| Script | Purpose |
|--------|---------|
| `cd deploy/gitops && make deploy ENV=local` | Brings up the full local stack: bootstrap (L0), Airbyte + Argo Workflows + the L2 system services, then the Insight umbrella chart (L3). Idempotent — re-running reconciles. Honours `$KUBECONFIG`. |
| `src/ingestion/reconcile-connectors.sh` | Declarative connector reconcile: `adopt` pre-existing Airbyte resources, then create / update / GC definitions, sources, connections, and per-connector sync CronWorkflows to match descriptor + Secret state. Idempotent; also runs in-cluster on a schedule via the reconcile CronWorkflow. (ClickHouse migrations are applied separately by the `clickhouse-migrate` Helm Hook Job on every install/upgrade — see §4.4.1. MariaDB schema is applied per-service by each backend service's own sea-orm `Migrator` at startup — see §4.4 and [ADR-0006](ADR/0006-service-owned-migrations.md). Per-service MariaDB databases are provisioned by the `mariadb-init-svcdbs` Helm Hook Job against the external `mariadb.host`.) |
| `src/ingestion/sync-all.sh` | Trigger Airbyte sync for all connections. Reads connection IDs from state, calls Airbyte API. Use after the first reconcile to start the first data load, or anytime to re-sync all sources. |

**First-time setup**:
1. Copy connector secret examples → fill credentials (`src/ingestion/secrets/connectors/`)
2. `cd deploy/gitops && make deploy ENV=local` — full stack deployment (answer the wizard prompts on first run)
3. `./src/ingestion/reconcile-connectors.sh` — connectors, connections, per-connector CronWorkflows (ClickHouse migrations run automatically via the `clickhouse-migrate` Hook Job during step 2's helm install)
4. `cd src/ingestion && ./sync-all.sh` — trigger first Airbyte sync for all connections

**Re-running**: `make deploy ENV=local` is idempotent — re-running on a converged cluster reconciles the stack in place.

This enables:
- Testing connector registration and sync execution
- Running full extract → transform pipeline via Argo DAG workflows
- Monitoring workflows via Argo UI
- Validating dbt models against real data
- Full backend API testing with OIDC disabled locally

### 4.3 Ultra-Light Connector Debugging

For rapid nocode connector iteration without the full Airbyte platform:
- Run connector Docker image directly with `source.sh`
- Commands: `check`, `discover`, `read`
- Mount custom manifest, pipe output to local destination

See [Airbyte Connector DESIGN](../../connector/specs/DESIGN.md) for detailed debugging workflows.

### 4.4 Schema Migrations

The project persists data in two stores with two different migration
mechanisms. **ClickHouse** schema is file-based and applied by the
`clickhouse-migrate` Helm Hook Job on every install/upgrade; **MariaDB**
schema is service-owned — each backend service carries its own embedded `Migrator` and
applies its migrations at startup (see [ADR-0006](ADR/0006-service-owned-migrations.md)).
The two paths diverge on bookkeeping because of different evolution
patterns.

#### 4.4.1 ClickHouse migrations

- **Location**: `src/ingestion/scripts/migrations/*.sql`
- **Naming**: `YYYYMMDDHHMMSS_<description>.sql` — timestamp prefix
  enforces chronological order under lexicographic glob
- **Runner**: the `clickhouse-migrate` Helm Hook Job
  (`post-install,post-upgrade`, `charts/insight/templates/clickhouse-migrate-job.yaml`)
  runs `src/ingestion/scripts/apply-ch-migrations.sh` in the toolbox image.
  It creates the `staging`/`silver`/app databases and the ADR-0007
  placeholders, then applies each migration in glob order against the
  external ClickHouse over its HTTP interface (`lib/ch-exec.sh`). (The
  legacy `init.sh` loop that `kubectl exec`'d into a bundled CH StatefulSet
  was retired with the StatefulSet in #1428.)
- **Bookkeeping**: none — every migration is re-run on every helm
  install/upgrade and therefore must be written idempotently
  (`CREATE ... IF NOT EXISTS`, `ALTER ... IF NOT EXISTS`, etc.)
- **Rationale**: analytics schema is mostly `CREATE TABLE` / `CREATE
  VIEW` statements where idempotency guards are free, so a
  `schema_migrations` bookkeeping table is unnecessary

#### 4.4.2 MariaDB migrations — service-owned

There is **no global MariaDB migration mechanism** in this project.
Each backend service that owns MariaDB tables carries its own
migrations inside the service codebase and applies them at startup.

- **analytics** (Rust): SeaORM `Migrator` at
  `src/backend/services/analytics/src/migration/`; tracker table
  `seaql_migrations` in the database referenced by
  `mariadb.database` (umbrella chart, default `insight`).
- **identity** (.NET 9): DbUp at
  `src/backend/services/identity/src/Insight.Identity.Infrastructure/Migrations/`
  (plain SQL files, applied via `MigrationRunner`); tracker table
  `SchemaVersions` (DbUp default) in the database referenced by
  `identity.databaseName` (umbrella chart, default `identity`).
- **Future backend services**: follow the same pattern — own
  `Migrations/` folder co-located with the service code, own tracker
  table in own database. Tooling flavour follows the service's
  language (SeaORM for Rust, DbUp for .NET).

All services invoke their migrator at startup before opening the HTTP
listener. This provides:

- **Co-location**: schema changes ship with the Rust code that depends
  on them; a service's DDL and its entity definitions never drift.
- **Database isolation**: each service lives in its own MariaDB
  database; cross-service table access is an explicit architectural
  decision, not an accident of shared-schema layout.
- **No ordering coupling between services**: no central bootstrap step
  must run before any service starts.

Infra responsibility (umbrella chart) stays minimal: MariaDB runs as an
external L2 release, and a Helm Hook Job
(`charts/insight/templates/mariadb-init-svcdbs-job.yaml`,
`pre-install,pre-upgrade`) dials `mariadb.host` and creates the
per-service databases (e.g. `CREATE DATABASE IF NOT EXISTS identity`)
and grants the app user access — idempotent (`IF NOT EXISTS`), so
helm upgrades are no-ops on this front. **Schema inside each
database** is the owning service's job.

See [ADR-0006](ADR/0006-service-owned-migrations.md) for the decision
record.

#### 4.4.3 One-shot data seeds

Operator-triggered data bootstraps (e.g. the identity-resolution
`persons` seed reading from ClickHouse `identity.identity_inputs`) are
not schema migrations and do not belong under the service's SeaORM
`Migrator`. They live alongside the owning service as stand-alone
scripts and are invoked explicitly by operators after migrations run
and the underlying data source is populated.

Currently:
- `src/backend/services/identity/seed/seed-persons.sh` + `seed-persons-from-identity-input.py` — identity-resolution's one-shot seed.

## 5. Additional Context

### 5.1 Superseded Components

This design supersedes:

| Component | Document | Reason |
|-----------|----------|--------|
| Custom Orchestrator | [Orchestrator PRD](../../../components/orchestrator/specs/PRD.md) | Replaced by Argo Workflows -- see [ADR-0002](ADR/0002-argo-over-kestra.md) |
| Custom Connector Framework | [Connector Framework DESIGN](../../connector/specs/DESIGN.md) | Replaced by Airbyte connector runtime |
| Stdout JSON Protocol | [ADR-0001](../../connector/specs/ADR/0001-connector-integration-protocol.md) | Replaced by Airbyte Protocol |

These documents remain as historical reference. The ingestion layer is the current and forward-looking approach.

## 6. Open Questions

### OQ-ING-01: ClickHouse Destination Normalization Mode

Airbyte's ClickHouse destination supports different normalization modes (raw, basic, etc.). Which mode should be used for Bronze table creation? Does the destination correctly handle `ReplacingMergeTree` with version columns?

### OQ-ING-02: ~~Kestra Flow Granularity~~ (RESOLVED)

Resolved by migration to Argo Workflows. Granularity: one CronWorkflow per (connector, tenant) pair. Reusable WorkflowTemplates (`airbyte-sync`, `dbt-run`, `ingestion-pipeline`) are shared across all tenants; tenant-specific CronWorkflows reside in `src/ingestion/workflows/{tenant_id}/`.

### OQ-ING-03: MariaDB Destination Use Cases

Which specific connectors or data types require MariaDB instead of ClickHouse as destination? This needs to be documented separately when the use cases are defined.

### OQ-ING-04: Gold Layer Ownership

Is the Gold layer part of the ingestion domain or a separate domain? Currently out of scope, but the boundary needs clarification.

### OQ-ING-05: Connector Package Versioning

How are connector packages versioned within the monorepo? Is there a version field in descriptor.yaml that gates deployment, or is Git commit the version?

## 7. Traceability

| Design Element | PRD Requirement |
|---------------|----------------|
| `cpt-insightspec-component-ing-airbyte` | `cpt-insightspec-fr-ing-airbyte-extract`, `cpt-insightspec-fr-ing-nocode-connector`, `cpt-insightspec-fr-ing-cdk-connector` |
| `cpt-insightspec-component-ing-argo` | `cpt-insightspec-fr-ing-kestra-scheduling`, `cpt-insightspec-fr-ing-kestra-dependency`, `cpt-insightspec-fr-ing-kestra-retry` |
| `cpt-insightspec-component-ing-dbt` | `cpt-insightspec-fr-ing-dbt-bronze-to-silver`, `cpt-insightspec-fr-ing-dbt-clickhouse`, `cpt-insightspec-fr-ing-silver-unified-schema` |
| `cpt-insightspec-component-ing-clickhouse` | `cpt-insightspec-fr-ing-bronze-storage`, `cpt-insightspec-fr-ing-clickhouse-destination` |
| `cpt-insightspec-component-ing-terraform` | `cpt-insightspec-fr-ing-terraform-connections` |
| `cpt-insightspec-principle-ing-tenant-isolation` | `cpt-insightspec-fr-ing-tenant-id`, `cpt-insightspec-nfr-ing-tenant-isolation` |
| `cpt-insightspec-principle-ing-package-self-contained` | `cpt-insightspec-fr-ing-package-structure`, `cpt-insightspec-fr-ing-package-monorepo` |
| `cpt-insightspec-constraint-ing-terraform-connections` | `cpt-insightspec-fr-ing-terraform-connections`, `cpt-insightspec-fr-ing-airbyte-api-custom` |

- **PRD**: [PRD.md](PRD.md)
- **ADR-0001**: [ADR/0001-kestra-over-airflow.md](ADR/0001-kestra-over-airflow.md) (superseded)
- **ADR-0002**: [ADR/0002-argo-over-kestra.md](ADR/0002-argo-over-kestra.md)
- **Airbyte Connector DESIGN**: [../../connector/specs/DESIGN.md](../../connector/specs/DESIGN.md)
- **Identity Resolution DESIGN**: [../../identity-resolution/specs/DESIGN.md](../../identity-resolution/specs/DESIGN.md) -- downstream consumer
