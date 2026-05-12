---
status: proposed
date: 2026-03-24
---

# Decomposition: Ingestion Layer

<!-- toc -->

- [1. Overview](#1-overview)
- [2. Entries](#2-entries)
  - [2.1 Local Infrastructure Stack — HIGH](#21-local-infrastructure-stack--high)
  - [2.2 Local Manifest Debugging — HIGH](#22-local-manifest-debugging--high)
  - [2.3 Manifest Upload Script — HIGH](#23-manifest-upload-script--high)
  - [2.4 Connection Management via API — HIGH](#24-connection-management-via-api--high)
  - [2.5 Argo Workflows Orchestration — HIGH](#25-argo-workflows-orchestration--high)
  - [2.6 dbt Project & Silver Union — HIGH](#26-dbt-project--silver-union--high)
  - [2.7 Reference Connector Package — M365 — MEDIUM](#27-reference-connector-package--m365--medium)
  - [2.8 K8s Secret Credential Resolution — HIGH](#28-k8s-secret-credential-resolution--high)
- [3. Feature Dependencies](#3-feature-dependencies)

<!-- /toc -->

## 1. Overview

The Ingestion Layer DESIGN is decomposed into seven features organized around deployment, tooling, and data flow concerns. The decomposition follows a dependency order: local infrastructure must exist before connectors can be tested; connectors must be registered before orchestration can run; dbt must be configured before Silver union models work.

**Decomposition Strategy**:
- Features grouped by operational boundary (infrastructure, tooling, data pipeline, configuration)
- Dependencies follow the natural setup order: infra → connectors → orchestration → transforms
- Each feature covers specific components, sequences, and requirements from DESIGN and PRD
- 100% coverage of all DESIGN elements verified
- Reference connector package (m365) serves as integration test across all features

**Key Architectural Decisions**:
- Silver layer union via dbt tags: each connector's `to_{domain}.sql` tagged with `silver:class_{domain}`, union models auto-discover by tag
- Connections managed via Airbyte API (`airbyte-toolkit/connect.sh`) with tenant YAML configs
- Per-tenant Argo CronWorkflows generated from connector `descriptor.yaml`
- Kind K8s cluster for local development (same Helm charts as production)
- `insight-toolbox` container runs all management scripts inside the cluster
- Auto-initialization on `./dev-up.sh` — no manual setup

## 2. Entries

**Overall implementation status:**

- [ ] `p1` - **ID**: `cpt-insightspec-status-overall`

### 2.1 [Local Infrastructure Stack](feature-local-infra/) — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-feature-local-infra`

- **Purpose**: Provide a fully automated local development environment via Kind K8s cluster that mirrors production topology. Running `./dev-up.sh` creates a working instance with ClickHouse, Airbyte, Argo Workflows, and all initialization — no manual configuration required.

- **Depends On**: None

- **Scope**:
  - Kind K8s cluster with all services: ClickHouse (Deployment + PVC), Airbyte (Helm chart), Argo Workflows (Helm chart)
  - `insight-toolbox` container that runs init scripts: create databases, register connectors, apply connections, create CronWorkflows
  - NodePort services for local access, PVC for data persistence
  - Automatic registration of all connector manifests from `src/ingestion/connectors/`
  - Automatic application of all Terraform connection configs from `src/ingestion/connections/`
  - Automatic loading of all Kestra flows

- **Out of scope**:
  - Production Kubernetes Helm deployment (separate feature later)
  - CI/CD pipelines
  - Monitoring and alerting

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-airbyte-extract`
  - [ ] `p1` - `cpt-insightspec-fr-ing-clickhouse-destination`
  - [ ] `p1` - `cpt-insightspec-fr-ing-bronze-storage`
  - [ ] `p1` - `cpt-insightspec-fr-ing-bronze-schema-native`
  - [ ] `p1` - `cpt-insightspec-fr-ing-secret-management`
  - [ ] `p1` - `cpt-insightspec-nfr-ing-error-isolation`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-no-custom-runtime`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-insightspec-constraint-ing-clickhouse-destination`

- **Domain Model Entities**:
  - AirbyteConnection
  - BronzeTable
  - ArgoWorkflow

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-ing-airbyte`
  - [ ] `p1` - `cpt-insightspec-component-ing-clickhouse`
  - [ ] `p1` - `cpt-insightspec-component-ing-argo`

- **API**:
  - Airbyte API (connector registration, connection creation)
  - Argo Workflows API (CronWorkflow management)
  - kubectl / helm (cluster management)

- **Sequences**:

  (none defined — new sequence for init flow to be added)

- **Data**:

  - [ ] `p1` - `cpt-insightspec-constraint-ing-clickhouse-destination`


### 2.2 [Local Manifest Debugging](feature-local-debug/) — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-feature-local-debug`

- **Purpose**: Enable rapid iteration on declarative connector manifests without the full Airbyte platform. Developer runs `source.sh check/discover/read` to validate manifests and inspect output locally.

- **Depends On**: None

- **Scope**:
  - `source.sh` script in `src/ingestion/tools/declarative-connector/` with commands: `check`, `discover`, `read`
  - Dockerfile and entrypoint.sh for wrapping `airbyte/source-declarative-manifest` image
  - Credentials via `.env.local` (AIRBYTE_CONFIG JSON)
  - Output to stdout only (no destination write)
  - Manifest validation (check command verifies manifest + credentials)
  - State management for incremental reads (state.json)

- **Out of scope**:
  - Destination write (no piping to ClickHouse/Postgres)
  - CDK connector debugging (Python)
  - Full platform integration testing

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-nocode-connector`
  - [ ] `p1` - `cpt-insightspec-fr-ing-incremental-sync`
  - [ ] `p1` - `cpt-insightspec-fr-ing-tenant-id`
  - [ ] `p1` - `cpt-insightspec-usecase-ing-local-debug`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-declarative-first`
  - [ ] `p1` - `cpt-insightspec-principle-abc-declarative-first`
  - [ ] `p1` - `cpt-insightspec-principle-abc-tenant-id-mandatory`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-insightspec-constraint-abc-airbyte-protocol`

- **Domain Model Entities**:
  - DeclarativeManifest
  - ConnectorPackage

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-abc-package`

- **API**:
  - `./source.sh check {connector}`
  - `./source.sh discover {connector}`
  - `./source.sh read {connector} {connection}`

- **Sequences**:

  - `cpt-insightspec-seq-abc-ultralight-debug`
  - `cpt-insightspec-seq-abc-nocode-execution`

- **Data**:

  (none — stdout only)


### 2.3 [Manifest Upload Script](feature-manifest-upload/) — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-feature-manifest-upload`

- **Purpose**: Provide a script to register or update a declarative connector manifest in a running Airbyte instance via its Public API. Used both by init containers and manually by developers.

- **Depends On**: `cpt-insightspec-feature-local-infra`

- **Scope**:
  - Script `src/ingestion/tools/upload-manifest.sh` (or similar)
  - Reads `connector.yaml` from connector package directory
  - Creates or updates source definition in Airbyte via POST/PATCH API
  - Supports all connectors in `src/ingestion/connectors/` via directory scan
  - Idempotent — safe to re-run

- **Out of scope**:
  - CDK connector registration (Docker image push)
  - Connection creation (that's feature 4 — Connection Management via API)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-nocode-connector`
  - [ ] `p2` - `cpt-insightspec-fr-ing-airbyte-api-custom`
  - [ ] `p1` - `cpt-insightspec-fr-ing-package-monorepo`
  - [ ] `p1` - `cpt-insightspec-usecase-ing-new-nocode-connector`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-declarative-first`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-insightspec-constraint-ing-no-external-registry`

- **Domain Model Entities**:
  - DeclarativeManifest
  - ConnectorPackage
  - Descriptor

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-ing-airbyte`

- **API**:
  - Airbyte Public API: POST /sources, PATCH /sources/{id}
  - `./upload-manifest.sh {connector}` or `./upload-manifest.sh --all`

- **Sequences**:

  (none defined — new sequence for manifest upload to be added)

- **Data**:

  (none)


### 2.4 [Connection Management via API](feature-connection-management/) — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-feature-terraform-connections`

- **Purpose**: Manage Airbyte connections (source → destination + catalog) as code via Airbyte API with tenant YAML configs. Running `./update-connections.sh {tenant}` creates all necessary connections.

- **Depends On**: `cpt-insightspec-feature-manifest-upload`

- **Scope**:
  - Tenant credential files in `connections/{tenant}.yaml` (gitignored)
  - `credentials.yaml.example` in each connector package (tracked)
  - `airbyte-toolkit/connect.sh` reads tenant YAML + connector `descriptor.yaml`
  - Creates source + destination + connection via Airbyte API
  - Connection state persisted as K8s ConfigMaps
  - Idempotent — safe to re-run

- **Out of scope**:
  - Custom connector registration (that's feature 3 — Manifest Upload)
  - CI/CD integration

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-terraform-connections`
  - [ ] `p1` - `cpt-insightspec-fr-ing-tenant-id`
  - [ ] `p1` - `cpt-insightspec-nfr-ing-tenant-isolation`
  - [ ] `p1` - `cpt-insightspec-usecase-ing-add-source-to-workspace`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-tenant-isolation`

- **Design Constraints Covered**:

  (none specific)

- **Domain Model Entities**:
  - AirbyteConnection
  - Descriptor

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-ing-airbyte`

- **API**:
  - Airbyte REST API (sources, destinations, connections)
  - `./update-connections.sh {tenant_id}`

- **Sequences**:

  - `cpt-insightspec-seq-ing-connection-apply`

- **Data**:

  (none — state in K8s ConfigMaps)


### 2.5 [Argo Workflows Orchestration](feature-argo-orchestration/) — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-feature-argo-orchestration`

- **Purpose**: Configure Argo Workflows to orchestrate the full ingestion pipeline: trigger Airbyte sync → wait for completion → run dbt transformations. Each connector defines a schedule in `descriptor.yaml`. CronWorkflows are generated from a shared template and applied per tenant.

- **Depends On**: `cpt-insightspec-feature-local-infra`, `cpt-insightspec-feature-terraform-connections`

- **Scope**:
  - Shared WorkflowTemplates: `airbyte-sync`, `dbt-run`, `ingestion-pipeline` (DAG)
  - Per-tenant CronWorkflows generated from `workflows/schedules/sync.yaml.tpl`
  - Schedule and `dbt_select` read from connector `descriptor.yaml`
  - `connection_id` resolved from state (K8s ConfigMap)
  - Retry policy via Argo `retryStrategy`
  - Script: `./update-workflows.sh {tenant_id}` or `./update-workflows.sh --all`

- **Out of scope**:
  - Event-driven triggers (webhook-based)
  - Monitoring and alerting beyond Argo UI

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-kestra-scheduling`
  - [ ] `p1` - `cpt-insightspec-fr-ing-kestra-dependency`
  - [ ] `p2` - `cpt-insightspec-fr-ing-kestra-retry`
  - [ ] `p1` - `cpt-insightspec-usecase-ing-scheduled-run`
  - [ ] `p2` - `cpt-insightspec-nfr-ing-observability`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-no-custom-runtime`

- **Design Constraints Covered**:

  (none specific — Argo is a design choice per ADR-0002)

- **Domain Model Entities**:
  - ArgoWorkflow
  - CronWorkflow

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-ing-argo`

- **API**:
  - Argo Workflows API / kubectl apply
  - `./update-workflows.sh {tenant_id}`

- **Sequences**:

  - `cpt-insightspec-seq-ing-scheduled-pipeline`

- **Data**:

  (none)


### 2.6 [dbt Project & Silver Union](feature-dbt-silver/) — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-feature-dbt-silver`

- **Purpose**: Configure the dbt project for Bronze-to-Silver transformations with automatic Silver layer union. Each connector package contains `to_{domain}.sql` models tagged with `silver:class_{domain}`. Shared union models auto-discover tagged sources via Jinja macros, producing unified Silver tables incrementally.

- **Depends On**: `cpt-insightspec-feature-local-infra`

- **Scope**:
  - Root `dbt_project.yml` at `src/ingestion/dbt/`
  - `profiles.yml` for dbt-clickhouse adapter (local + production targets)
  - Per-connector dbt models in `src/ingestion/connectors/{class}/{source}/dbt/to_{domain}.sql`
  - Tag convention: `silver:class_{domain}` on each connector model
  - Shared union models in `src/ingestion/dbt/silver/class_{domain}.sql` — auto-discover by tag via Jinja macro
  - `schema.yml` with `not_null` test on `tenant_id` for all Silver tables
  - Materialization strategy for Silver tables (incremental on ClickHouse)
  - ClickHouse engine settings (ReplacingMergeTree, ORDER BY, PARTITION BY)
  - Shared macros: tag-based union, `tenant_id` validation

- **Out of scope**:
  - Gold layer transformations
  - Identity resolution (Silver step 2)
  - dbt Cloud integration

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-dbt-bronze-to-silver`
  - [ ] `p1` - `cpt-insightspec-fr-ing-dbt-clickhouse`
  - [ ] `p1` - `cpt-insightspec-fr-ing-silver-unified-schema`
  - [ ] `p1` - `cpt-insightspec-fr-ing-tenant-id`
  - [ ] `p1` - `cpt-insightspec-nfr-ing-idempotency`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-silver-at-design-time`
  - [ ] `p1` - `cpt-insightspec-principle-abc-silver-targets-known`
  - [ ] `p1` - `cpt-insightspec-principle-ing-package-self-contained`
  - [ ] `p1` - `cpt-insightspec-principle-abc-package-self-contained`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-insightspec-constraint-ing-monorepo`

- **Domain Model Entities**:
  - DbtModels
  - SilverTable
  - BronzeTable
  - Descriptor

- **Database Tables**:

  - [ ] `p1` - `cpt-insightspec-dbtable-cn-bronze`
  - [ ] `p1` - `cpt-insightspec-dbtable-cn-staging`
  - [ ] `p1` - `cpt-insightspec-dbtable-cn-silver`

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-ing-dbt`

- **API**:
  - `dbt run --select tag:silver`
  - `dbt test --select tag:silver`

- **Sequences**:

  (dbt execution is part of `cpt-insightspec-seq-ing-scheduled-pipeline`)

- **Data**:

  - [ ] `p1` - `cpt-insightspec-constraint-ing-clickhouse-destination`


### 2.7 [Reference Connector Package — M365](feature-ref-m365/) — MEDIUM

- [ ] `p2` - **ID**: `cpt-insightspec-feature-ref-m365`

- **Purpose**: Provide a complete, working connector package for Microsoft 365 (email activity + teams activity) as the reference implementation. Demonstrates all packaging conventions: declarative manifest with `tenant_id` injection, descriptor YAML, dbt models with Silver tags, and integration with the full pipeline.

- **Depends On**: `cpt-insightspec-feature-local-debug`, `cpt-insightspec-feature-dbt-silver`

- **Scope**:
  - `src/ingestion/connectors/collaboration/m365/connector.yaml` — declarative manifest (OAuth2, pagination, incremental, `tenant_id` via AddFields)
  - `src/ingestion/connectors/collaboration/m365/descriptor.yaml` — package metadata (silver_targets, streams)
  - `src/ingestion/connectors/collaboration/m365/dbt/to_comms_events.sql` — Bronze → Silver transform with `silver:class_comms_events` tag
  - `src/ingestion/connectors/collaboration/m365/dbt/schema.yml` — column docs + tests
  - `src/ingestion/connectors/collaboration/m365/.env.local.example` — credential template
  - Terraform connection config for m365
  - Kestra flow template for m365

- **Out of scope**:
  - Other connectors (GitHub, GitLab, Jira, etc.)
  - CDK connector examples

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-package-structure`
  - [ ] `p1` - `cpt-insightspec-fr-ing-nocode-connector`
  - [ ] `p1` - `cpt-insightspec-fr-ing-tenant-id`
  - [ ] `p1` - `cpt-insightspec-fr-ing-incremental-sync`
  - [ ] `p1` - `cpt-insightspec-usecase-ing-new-nocode-connector`
  - [ ] `p1` - `cpt-insightspec-contract-ing-airbyte-protocol`
  - [ ] `p1` - `cpt-insightspec-contract-ing-dbt-contracts`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-declarative-first`
  - [ ] `p1` - `cpt-insightspec-principle-ing-package-self-contained`
  - [ ] `p1` - `cpt-insightspec-principle-abc-tenant-id-mandatory`

- **Design Constraints Covered**:

  - [ ] `p1` - `cpt-insightspec-constraint-abc-airbyte-protocol`
  - [ ] `p1` - `cpt-insightspec-constraint-abc-monorepo`
  - [ ] `p1` - `cpt-insightspec-constraint-ing-monorepo`

- **Domain Model Entities**:
  - ConnectorPackage
  - DeclarativeManifest
  - Descriptor
  - DbtModels

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-abc-package`
  - [ ] `p1` - `cpt-insightspec-component-ing-airbyte`
  - [ ] `p1` - `cpt-insightspec-component-ing-dbt`

- **API**:
  - All connector commands (check, discover, read)
  - dbt run/test for m365 models

- **Sequences**:

  - `cpt-insightspec-seq-abc-nocode-execution`

- **Data**:

  - [ ] `p1` - `cpt-insightspec-contract-ing-dbt-contracts`


### 2.8 [K8s Secret Credential Resolution](feature-k8s-secret-credentials/) — HIGH

- [ ] `p1` - **ID**: `cpt-insightspec-feature-k8s-secret-credentials`

- **Purpose**: Enable `airbyte-toolkit/connect.sh` to discover and resolve connector credentials from Kubernetes Secrets via label-based discovery, replacing inline plaintext credentials in tenant YAML. Consumers manage secrets through their own K8s secret infrastructure (Vault + ESO, Sealed Secrets, manual).

- **Depends On**: `cpt-insightspec-feature-terraform-connections`

- **Scope**:
  - `src/ingestion/airbyte-toolkit/connect.sh` — Secret discovery, credential merge, backward compatibility
  - `src/ingestion/connections/example-tenant.yaml` — updated to remove inline credentials
  - `src/ingestion/connectors/*/README.md` — per-connector K8s Secret specification (7 connectors)

- **Out of scope**:
  - ClickHouse destination credentials (separate concern)
  - Secret creation tooling (consumer responsibility)
  - Vault/ESO integration (transparent — produces K8s Secrets)

- **Requirements Covered**:

  - [ ] `p1` - `cpt-insightspec-fr-ing-secret-management`

- **Design Principles Covered**:

  - [ ] `p1` - `cpt-insightspec-principle-ing-tenant-isolation`

- **Design Components**:

  - [ ] `p1` - `cpt-insightspec-component-ing-airbyte`
  - [ ] `p1` - `cpt-insightspec-component-ing-terraform`

- **ADR**:

  - `cpt-insightspec-adr-k8s-secrets-credentials`

---

## 3. Feature Dependencies

```text
cpt-insightspec-feature-local-infra
    ↓
    ├─→ cpt-insightspec-feature-manifest-upload
    │       ↓
    │       └─→ cpt-insightspec-feature-terraform-connections
    │               ↓
    │               ├─→ cpt-insightspec-feature-argo-orchestration
    │               └─→ cpt-insightspec-feature-k8s-secret-credentials
    ├─→ cpt-insightspec-feature-dbt-silver
    │       ↓
    │       └─→ cpt-insightspec-feature-ref-m365
    └─→ (none)

cpt-insightspec-feature-local-debug  (independent — no platform dependency)
    ↓
    └─→ cpt-insightspec-feature-ref-m365
```

**Dependency Rationale**:

- `cpt-insightspec-feature-manifest-upload` requires `cpt-insightspec-feature-local-infra`: needs a running Airbyte instance to upload manifests to
- `cpt-insightspec-feature-terraform-connections` requires `cpt-insightspec-feature-manifest-upload`: connections reference source definitions that must be registered first
- `cpt-insightspec-feature-argo-orchestration` requires `cpt-insightspec-feature-local-infra` and `cpt-insightspec-feature-terraform-connections`: CronWorkflows trigger syncs on existing connections
- `cpt-insightspec-feature-k8s-secret-credentials` requires `cpt-insightspec-feature-terraform-connections`: K8s Secret resolution modifies the connection management script
- `cpt-insightspec-feature-dbt-silver` requires `cpt-insightspec-feature-local-infra`: dbt needs ClickHouse to be running
- `cpt-insightspec-feature-ref-m365` requires `cpt-insightspec-feature-local-debug` and `cpt-insightspec-feature-dbt-silver`: reference package needs both debugging tools and dbt project structure
- `cpt-insightspec-feature-local-debug` is independent — runs standalone Docker containers without the platform

**Parallel tracks**:
- Track A (platform): local-infra → manifest-upload → connection-management → argo-orchestration
- Track A' (secrets): connection-management → k8s-secret-credentials
- Track B (data): local-infra → dbt-silver → ref-m365
- Track C (standalone): local-debug → ref-m365
