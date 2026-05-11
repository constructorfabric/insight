---
cpt:
  artifact: DESIGN
  system: insightspec
  version: "1.1"
---

# Technical Design — Airbyte Toolkit


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
  - [3.9 Reconciliation Model](#39-reconciliation-model)
  - [3.10 Adoption (one-shot)](#310-adoption-one-shot)
  - [3.11 Naming Convention](#311-naming-convention)
  - [3.12 Secret Validation](#312-secret-validation)
  - [3.13 Argo Integration](#313-argo-integration)
  - [3.14 Cron Self-Run + Leak Guarantees](#314-cron-self-run--leak-guarantees)
  - [3.15 File Log Destination](#315-file-log-destination)
- [4. Additional context](#4-additional-context)
  - [Migration from old scripts](#migration-from-old-scripts)
  - [State library API](#state-library-api)
- [5. Traceability](#5-traceability)
  - [PRD §5.6 (Reconcile Engine Phase 2) → DESIGN](#prd-56-reconcile-engine-phase-2--design)
  - [Changelog](#changelog)

<!-- /toc -->

- [ ] `p3` - **ID**: `cpt-insightspec-design-airbyte-toolkit`
## 1. Architecture Overview

### 1.1 Architectural Vision

Airbyte Toolkit is a self-contained module within `src/ingestion/airbyte-toolkit/` that owns all Airbyte API interactions and resource state. It exposes shell scripts as the public interface and stores state in a single hierarchical YAML file.

The design prioritizes deterministic state access: every Airbyte resource ID is reachable via a fixed YAML path known at call time, with no string concatenation, prefix matching, or naming convention translation. The module auto-detects its runtime environment (host vs in-cluster) and resolves API endpoints and credentials accordingly.

All operations are idempotent. Creating a resource that already exists in state updates it; deleting a resource not found in Airbyte cleans the stale state entry. This makes the toolkit safe to call repeatedly from CI/CD or manual recovery flows.

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|------------------|
| `cpt-insightspec-fr-single-state` | One file at `airbyte-toolkit/state.yaml`, all scripts read/write it |
| `cpt-insightspec-fr-hierarchical-state` | YAML tree with separate levels for connector name and source-id |
| `cpt-insightspec-fr-tenant-key` | Tenant key stored as-is from config filename |
| `cpt-insightspec-fr-idempotent` | All commands use create-or-update pattern with state-tracked IDs |
| `cpt-insightspec-fr-register-definitions` | `register.sh` writes to `definitions.{connector}.id` |
| `cpt-insightspec-fr-create-connections` | `connect.sh` writes to `tenants.{tenant}.connectors.{connector}.{source_id}` |
| `cpt-insightspec-fr-version-driven-reconcile` | `descriptor.yaml.version` ↔ `definition.declarativeManifest.description` (nocode) or `dockerImageTag` (CDK); reconcile-engine compares, republishes only on mismatch |
| `cpt-insightspec-fr-adopt-legacy-resources` | `adopt-pass` annotates description + `connection.tags` on existing resources without recreate; ref-count-zero duplicate definitions deleted |
| `cpt-insightspec-fr-orphan-gc` | reconcile-engine sweeps Airbyte by `insight` membership tag; deletes resources without matching K8s Secret unless `--no-gc` |
| `cpt-insightspec-fr-state-preserved-on-breaking-change` | breaking schema change → `state_export → delete → create → state_import` via `/api/v1/state/{get,create_or_update}`; non-breaking → `connections/update` |
| `cpt-insightspec-fr-secret-validation` | `secret-validator` (read-only) checks K8s Secret schema vs `secrets/connectors/*.yaml.example` and OnePasswordItem CR ↔ child Secret label/annotation drift |
| `cpt-insightspec-fr-cli-surface` | single `reconcile-connectors.sh [adopt\|reconcile] [--dry-run] [--connector <name>] [--no-gc]` entrypoint; legacy scripts removed |
| `cpt-insightspec-fr-jwt-auth` | env-resolver mints/refreshes a JWT for the Airbyte API; both host and in-cluster runtimes share the same auth path (no per-script auth code) |

#### ADR References

| ADR | Subject | Drives |
|-----|---------|--------|
| `cpt-insightspec-adr-version-driven-reconcile` | descriptor.yaml.version is the single reconcile driver | §3.2 reconcile-engine, §3.9 Reconciliation Model |
| `cpt-insightspec-adr-adoption-of-existing-resources` | tag-based adoption preserves sync state on legacy clusters | §3.2 adopt-pass, §3.10 Adoption |
| `cpt-insightspec-adr-credential-rotation-no-env` | sources/update on cfg-hash mismatch (not env-vars / SecretPersistence) | §3.2 reconcile-engine, §3.12 Secret Validation |
| `cpt-insightspec-adr-cluster-config-via-configmap` | tenant_id from ConfigMap `insight-config` (or env override) | §3.2 secret-discovery, §3.11 Naming Convention |
| `cpt-insightspec-adr-connection-name-as-argo-identifier` | per-connector CronWorkflow stores `connection_name`; resolver init-step maps to `connection_id` at submit time | §3.2 argo-name-resolver, §3.13 Argo Integration |
| `cpt-insightspec-adr-cron-self-run-with-file-persistent-logs` | cluster-level Argo CronWorkflow drives reconcile; PVC-backed daily-rotated log file | §3.2 reconcile-cronworkflow, §3.14 Cron Self-Run, §3.15 File Log Destination |
| `cpt-insightspec-adr-required-fields-in-descriptor-not-example` | required Secret fields declared in `descriptor.yaml.secret.required_fields`, not in example annotations | §3.12 Secret Validation |
| `cpt-insightspec-adr-auto-trigger-sync-on-data-change` | one-shot `airbyte-sync` Workflow fires only on data-affecting reconcile actions | §3.2 argo-sync-trigger, §3.13 Argo Integration |
| `cpt-insightspec-adr-airbyte-workspace-as-namespace` | Insight definitions live in one Airbyte workspace, identified by `custom: true` | §3.2 reconcile-engine, §3.9 Reconciliation Model, §3.11 Naming Convention |
| `cpt-insightspec-adr-nocode-via-builder-projects` | nocode definitions registered via `connector_builder_projects` (create/publish/update_active_manifest); CDK keeps `create_custom` | §3.2 registrar, §3.2 reconcile-engine, §3.9 Reconciliation Model |
| `cpt-insightspec-adr-cdk-prebuilt-images` | CDK connectors carry the full Docker image reference in `descriptor.cdk_image`; reconcile splits it into `dockerRepository`/`dockerImageTag` per Docker reference grammar — no derivation, no convention, no env var; never builds at runtime | §3.2 reconcile-engine, §3.9 Reconciliation Model |

#### NFR Allocation

| NFR ID | NFR Summary | Allocated To | Design Response | Verification Approach |
|--------|-------------|--------------|-----------------|----------------------|
| `cpt-insightspec-nfr-dual-runtime` | Host and in-cluster execution | `lib/env.sh` | Auto-detect via service account token presence; set API URL and auth accordingly | Manual test on host + in-cluster job |

### 1.3 Architecture Layers

```
src/ingestion/airbyte-toolkit/
├── state.yaml          ← single state file (gitignored)
├── lib/
│   ├── state.sh        ← state read/write library
│   ├── env.sh                          ← environment resolution (API URL, JWT, workspace)
│   └── host-side-prerequisites.sh      ← yq/jq/PyYAML auto-install +
│                                         airbyte-server port-forward
│                                         (no-op in-cluster)
├── register.sh         ← register source definitions
├── connect.sh          ← create sources + connections per tenant
├── sync-state.sh       ← rebuild state from Airbyte API
└── cleanup.sh          ← delete resources by state
```

- [ ] `p3` - **ID**: `cpt-insightspec-tech-toolkit-layout`

| Layer | Responsibility | Technology |
|-------|---------------|------------|
| CLI | User-facing scripts, argument parsing | Bash |
| Library | State I/O, environment resolution, API helpers | Bash + Python (inline) |
| State | Persistent storage of Airbyte resource IDs | YAML file + K8s ConfigMap |
| External | Airbyte REST API, K8s API | HTTP/JSON, kubectl |

## 2. Principles & Constraints

### 2.1 Design Principles

#### Deterministic access paths

- [ ] `p2` - **ID**: `cpt-insightspec-principle-deterministic-paths`

Every resource ID in the state file is accessed via a path that can be constructed from the operation's input parameters alone. No searching, no iteration, no pattern matching.

#### No string concatenation for keys

- [ ] `p2` - **ID**: `cpt-insightspec-principle-no-concat`

Composite identity (connector + source-id, connector + tenant) is expressed as nested YAML levels, never as concatenated strings. `bamboohr.bamboohr-main` is two map levels, not `bamboohr-bamboohr-main` as one key.

#### State as source of truth

- [ ] `p2` - **ID**: `cpt-insightspec-principle-state-truth`

Scripts identify Airbyte resources by UUID from state — never by name. If the UUID returns 404 from Airbyte, the stale entry is removed and the resource is recreated.

### 2.2 Constraints

#### Single workspace

- [ ] `p2` - **ID**: `cpt-insightspec-constraint-single-workspace`

The toolkit assumes one Airbyte workspace per cluster (the default workspace created by the Helm chart). Multi-workspace support is not planned.

#### Shared destination

- [ ] `p2` - **ID**: `cpt-insightspec-constraint-shared-dest`

All connections use a single shared ClickHouse destination. Per-connector Bronze databases are controlled via `namespaceFormat` on the connection, not via separate destinations.

## 3. Technical Architecture

### 3.1 Domain Model

**Core Entities**:

| Entity | Description | Identity |
|--------|-------------|----------|
| Definition | Registered connector type in Airbyte | `definitions.{connector}.id` |
| Source | Configured connector instance with credentials | `tenants.{tenant}.connectors.{connector}.{source_id}.source_id` |
| Connection | Source-to-destination link with stream selection | `tenants.{tenant}.connectors.{connector}.{source_id}.connection_id` |
| Destination | Shared ClickHouse target | `destinations.{name}.id` |

**Relationships**:
- Definition 1→N Source: each source references a definition
- Source 1→1 Connection: each source has exactly one connection
- Connection N→1 Destination: all connections share one destination

### 3.2 Component Model

#### State Manager

- [ ] `p2` - **ID**: `cpt-insightspec-component-state-manager`

##### Why this component exists

Provides atomic read/write access to the state file. All other components use it instead of accessing the file directly.

##### Responsibility scope

- Read/write individual values by YAML path.
- Read entire state for iteration.
- Persist to file and (optionally) K8s ConfigMap.
- Initialize empty state file if missing.

##### Responsibility boundaries

- Does NOT interact with Airbyte API.
- Does NOT validate that IDs exist in Airbyte.

##### Related components (by ID)

None — State Manager is a leaf dependency used by all other components.

#### Environment Resolver

- [ ] `p2` - **ID**: `cpt-insightspec-component-env-resolver`

##### Why this component exists

Centralizes runtime detection and credential resolution. Eliminates duplicated env resolution across scripts.

##### Responsibility scope

- Detect host vs in-cluster runtime.
- Read Airbyte auth secrets from K8s.
- Mint JWT token for API access.
- Resolve workspace ID.
- Export: `AIRBYTE_API`, `AIRBYTE_TOKEN`, `WORKSPACE_ID`.

##### Responsibility boundaries

- Does NOT manage state.
- Does NOT create Airbyte resources.

##### Related components (by ID)

- `cpt-insightspec-component-state-manager` — depends on (reads `workspace_id` from state for caching)

#### Definition Registrar

- [ ] `p2` - **ID**: `cpt-insightspec-component-registrar`

##### Why this component exists

Registers connector manifests as Airbyte source definitions.

##### Responsibility scope

- Read `connector.yaml` manifests from `connectors/` directory.
- Create or update Airbyte source definitions via API.
- Store `definition_id` in state via State Manager.

##### Responsibility boundaries

- Does NOT create sources or connections.
- Does NOT read tenant configs.

##### Related components (by ID)

- `cpt-insightspec-component-state-manager` — depends on (writes definition IDs)
- `cpt-insightspec-component-env-resolver` — depends on (API credentials)

#### Connection Manager

- [ ] `p2` - **ID**: `cpt-insightspec-component-connection-mgr`

##### Why this component exists

Creates and updates sources, destinations, and connections for a tenant.

##### Responsibility scope

- Read tenant config (`connections/{tenant}.yaml`).
- Discover K8s Secrets for connector credentials.
- Create/update shared ClickHouse destination.
- Create/update sources (one per connector + source-id).
- Discover schema from source.
- Create/update connections with stream selection.
- Store all IDs in state via State Manager.
- Create Bronze databases in ClickHouse.

##### Responsibility boundaries

- Does NOT register definitions (assumes they exist in state).
- Does NOT manage Argo workflows.

##### Related components (by ID)

- `cpt-insightspec-component-state-manager` — depends on (reads definitions, writes sources/connections)
- `cpt-insightspec-component-env-resolver` — depends on (API credentials)
- `cpt-insightspec-component-registrar` — depends on (definition IDs must exist)

#### Reconcile Engine

- [ ] `p1` - **ID**: `cpt-insightspec-component-reconcile-engine`

##### Why this component exists

Drives Airbyte resources (definitions, sources, connections) into the desired state declared by `connectors/*/descriptor.yaml` + K8s Secrets, idempotently and without losing accumulated sync state. Replaces the create-or-update logic previously scattered across `register.sh` and `connect.sh`.

##### Responsibility scope

- Owns the diff & apply loop across three layers (definition / source / connection).
- Decides when to republish a definition: only when `descriptor.yaml.version` ≠ `definition.declarativeManifest.description` (nocode) or `dockerImageTag` (CDK).
- Performs idempotent `sources/update` per Secret (`sources` are append-tolerant; connection state is preserved).
- Decides when to recreate a connection: only on breaking syncCatalog drift; uses `cpt-insightspec-seq-breaking-change-recreate-with-state` to preserve cursors via `/api/v1/state/{get,create_or_update}`.
- Creates/recreates Airbyte connections with `scheduleType=manual`; the per-connector Argo CronWorkflow (rendered by `cpt-insightspec-component-argo-cronworkflow-renderer`) is the sole sync scheduler. Airbyte's own Temporal scheduler must not fire syncs in parallel with Argo, or Bronze rows land without dbt → Silver.
- Drives orphan GC by `insight` membership tag (skipped under `--no-gc`).
- Reports per-connector outcome: `created` | `updated` | `no-op` | `recreated` | `deleted` | `skipped`.

##### Responsibility boundaries

- Does NOT author connector manifests (`connectors/*/connector.yaml` is owned by connector authors).
- Does NOT manage Argo workflows.
- Does NOT modify K8s Secrets.
- Does NOT keep parallel local state — Airbyte is the source of truth post-refactor (no more `state.yaml` / `airbyte-state` ConfigMap).

##### Related components (by ID)

- `cpt-insightspec-component-secret-discovery` — depends on (input desired state)
- `cpt-insightspec-component-env-resolver` — depends on (API credentials)
- `cpt-insightspec-component-adopt-pass` — runs before reconcile on legacy clusters

#### Secret Discovery

- [ ] `p2` - **ID**: `cpt-insightspec-component-secret-discovery`

##### Why this component exists

Computes the desired state from K8s Secrets and `descriptor.yaml` files. The reconcile engine consumes its output.

##### Responsibility scope

- Lists Secrets in `data` namespace with label `app.kubernetes.io/part-of=insight`.
- Reads annotations `insight.cyberfabric.com/connector` and `insight.cyberfabric.com/source-id`; pairs each Secret with `connectors/<connector>/descriptor.yaml`.
- Computes `cfg_hash = sha256(canonical(secret.data))` per Secret.
- Resolves `tenant_id` from ConfigMap `insight-config` (or env `INSIGHT_TENANT_ID`).
- On missing/invalid metadata: WARN + skip (per connector, never abort).

##### Responsibility boundaries

- Does NOT decode 1Password vault items (operator's job).
- Does NOT apply changes — pure read.
- Does NOT validate Secret schema against `connection_specification` — that is `secret-validator`'s job.

##### Related components (by ID)

- `cpt-insightspec-component-reconcile-engine` — consumer
- `cpt-insightspec-component-adopt-pass` — same input source

#### Adopt Pass

- [ ] `p2` - **ID**: `cpt-insightspec-component-adopt-pass`

##### Why this component exists

Migrates legacy clusters whose Airbyte resources lack the post-refactor metadata (no description on definitions, no tags on connections), without recreating any source or connection — preserving all accumulated sync state.

##### Responsibility scope

- For each Secret matched to an existing source by name pattern: patch `definition.declarativeManifest.description` to descriptor version, patch `connection.tags` to `[insight, cfg-hash:<sha256(secret.data)>]`.
- Identify duplicate definitions per connector name and delete only those with `ref_count == 0`.
- Idempotent: running twice is a no-op on the already-annotated set.
- Operates under `--dry-run` to preview changes before any state-changing call.

##### Responsibility boundaries

- Does NOT create new sources or connections (reconcile mode does that).
- Does NOT delete resources whose Secret is missing — that is reconcile's GC sweep.
- Does NOT push credentials via `sources/update` — only metadata patches.

##### Related components (by ID)

- `cpt-insightspec-component-secret-discovery` — input
- `cpt-insightspec-component-reconcile-engine` — runs after adopt on legacy clusters

#### Secret Validator

- [ ] `p2` - **ID**: `cpt-insightspec-component-secret-validator`

##### Why this component exists

Detects drift between the cluster's K8s Secrets and what `secrets/connectors/*.yaml.example` declares as the canonical schema, and surfaces label/annotation drift between OnePasswordItem CRs and their child Secrets — 1Password operator copies labels but NOT custom annotations, so misconfigured items can silently fall out of discovery.

##### Responsibility scope

- Reads each `secrets/connectors/*.yaml.example` to learn required `stringData` keys and required labels/annotations.
- Reads each Secret in `data` namespace; reads OnePasswordItem CRs in `data` namespace.
- Compares per connector: required `stringData` keys, required labels, required annotations, OnePasswordItem CR ↔ child Secret label/annotation parity.
- Pure read — no `kubectl apply`, `kubectl patch`, or `kubectl annotate` calls.
- Exit codes: `0` if no errors, `1` if errors, `2` reserved for environmental failures.

##### Responsibility boundaries

- Does NOT modify any cluster object.
- Does NOT validate connector behavior or live API access.
- Does NOT compare Secret values (avoids credential leakage in output).

##### Related components (by ID)

- `cpt-insightspec-component-secret-discovery` — same Secret enumeration; validator runs first in `run-init.sh`

#### Argo CronWorkflow Renderer

- [ ] `p1` - **ID**: `cpt-insightspec-component-argo-cronworkflow-renderer`

##### Why this component exists

Per-connector Argo `CronWorkflow` objects must be created/updated/deleted in lockstep with the desired set of K8s Secrets, and their spec must encode `connection_name` (not `connection_id`) so they survive connection recreate-with-state. Without an explicit renderer, every reconcile would inline templated YAML in shell, which Decision #9 forbids.

##### Responsibility scope

- Renders `templates/cron-workflow.yaml.tpl` with `{connector, connection_name, schedule, tenant_id}`.
- Idempotently applies the result via `kubectl apply -f -`.
- Per-connector name pattern `${connector}-${tenant}-sync`.
- Schedule precedence (resolved before render): Secret annotation `insight.cyberfabric.com/schedule` > `descriptor.yaml.schedule` > default `0 0 * * *`.

##### Responsibility boundaries

- Does NOT delete CronWorkflows on Secret-missing — that is `cpt-insightspec-component-reconcile-engine`'s cascade.
- Does NOT submit one-shot sync Workflows — that is `cpt-insightspec-component-argo-sync-trigger`.

##### Related components (by ID)

- `cpt-insightspec-component-reconcile-engine` — caller; passes desired set
- `cpt-insightspec-component-argo-name-resolver` — consumed at submit time by the WorkflowTemplate the rendered CronWorkflow references

#### Argo Sync Trigger

- [ ] `p1` - **ID**: `cpt-insightspec-component-argo-sync-trigger`

##### Why this component exists

A reconcile iteration that detects a data-affecting change must not wait for the next CronWorkflow tick to re-sync. Submitting a one-shot `airbyte-sync` Workflow decouples data freshness from cron latency.

##### Responsibility scope

- Renders `templates/sync-trigger.yaml.tpl` with `{connector, connection_name, tenant_id}`.
- Submits the Workflow via `kubectl create -f -` (uses `generateName`).
- Triggered ONLY on data-affecting changes: descriptor.version bump, Secret cfg-hash mismatch, new connector/connection, recreate-with-state on breaking syncCatalog drift.
- NOT triggered on tag-only patches or `definition.description`-only patches.

##### Responsibility boundaries

- Does NOT manage CronWorkflow lifecycle (renderer's job).
- Does NOT touch Airbyte API directly — submission is via the Workflow which calls `cpt-insightspec-component-argo-name-resolver` first.

##### Related components (by ID)

- `cpt-insightspec-component-reconcile-engine` — decides when to fire
- `cpt-insightspec-component-argo-name-resolver` — resolves name → id at Workflow submit time

#### Argo Name Resolver

- [ ] `p1` - **ID**: `cpt-insightspec-component-argo-name-resolver`

##### Why this component exists

Recreate-with-state assigns a new connection UUID. CronWorkflow specs that hard-code UUIDs become stale after recreate. Storing `connection_name` in the spec and resolving it to `connection_id` at submit time keeps the spec valid across recreates.

##### Responsibility scope

- Init-step in the `airbyte-sync` WorkflowTemplate.
- Calls `ab_list_connections` and matches `connection_name` (pattern `{connector}-{source_id}-to-clickhouse-{tenant}`) to a record.
- Outputs `connection_id` for the next workflow step.
- Lookup miss fails the Workflow with `ERROR: connection name not found`.

##### Responsibility boundaries

- Does NOT cache results across runs (the cluster has no shared state for this).
- Does NOT mutate any Airbyte resource.

##### Related components (by ID)

- `cpt-insightspec-component-connection-mgr` — sibling consumer of `lib/airbyte.sh` (`ab_list_connections`)
- `cpt-insightspec-component-argo-cronworkflow-renderer` — produces the spec that triggers this resolver

#### Reconcile CronWorkflow

- [ ] `p1` - **ID**: `cpt-insightspec-component-reconcile-cronworkflow`

##### Why this component exists

The reconcile loop runs autonomously inside the cluster on a `*/15` schedule, removing the Kestra/external-orchestrator dependency. Required RBAC and PVC come with the chart so adding a new cluster does not need extra ops steps.

##### Responsibility scope

- Cluster-level Argo `CronWorkflow` named `insight-reconcile-loop` running `bash src/ingestion/reconcile-connectors/main.sh`.
- Schedule: Helm value `ingestion.reconcile.schedule` (default `*/15 * * * *`).
- Toolbox image with `kubectl`, `python3`, `pyyaml`, `node`.
- ServiceAccount + RBAC: read `secrets`/`onepassworditems`/`configmaps`; create/get/delete `workflows.argoproj.io`/`cronworkflows.argoproj.io`; in-cluster Airbyte API access.
- Bootstrap on a fresh cluster: creates N connectors and submits N parallel sync Workflows in one tick; Airbyte queues; no app-level rate-limiting.

##### Responsibility boundaries

- Does NOT include any per-connector logic — the entrypoint is `main.sh`, which fans out per-connector calls.
- Does NOT keep state on disk between runs; the cron pod exits per run.

##### Related components (by ID)

- `cpt-insightspec-component-reconcile-engine` — the actual loop body invoked per tick
- `cpt-insightspec-component-reconcile-file-logger` — durable logs across pod restarts

#### Reconcile File Logger

- [ ] `p2` - **ID**: `cpt-insightspec-component-reconcile-file-logger`

##### Why this component exists

`kubectl logs` for a CronWorkflow is bounded by pod lifetime. A durable change history across pod restarts requires a file destination. Quiet-run runs must emit ZERO file lines so the log file does not grow when nothing happened.

##### Responsibility scope

- In-cluster destination: `/var/log/insight/reconcile-${YYYY-MM-DD}.log` on PVC `insight-reconcile-logs` (default 5Gi via `ingestion.reconcile.logs.size`; storage class via `ingestion.reconcile.logs.storageClass`).
- Local destination: `${XDG_STATE_HOME:-$HOME/.local/state}/insight/reconcile-${YYYY-MM-DD}.log` (append).
- Format: text. One line per change/error: `${TIMESTAMP_UTC} [LEVEL] ${MSG}`.
- Quiet-run policy: ZERO file lines on no-op runs.
- Every run emits ONE stdout summary line for `kubectl logs` sanity.
- Boundary: `lib/log.sh` exposes `log_init`, `log_line`, `log_run_summary`, `log_close`.

##### Responsibility boundaries

- Does NOT rotate by size — only daily filename rotation.
- Does NOT ship logs anywhere external (no Loki, no S3).
- Does NOT log values that may contain secrets.

##### Related components (by ID)

- `cpt-insightspec-component-reconcile-engine` — caller
- `cpt-insightspec-component-reconcile-cronworkflow` — mounts the PVC

### 3.3 API Contracts

- [ ] `p2` - **ID**: `cpt-insightspec-interface-state-yaml`

- **Contracts**: `cpt-insightspec-contract-airbyte-api`
- **Technology**: YAML file (state format)

**State file schema** (`airbyte-toolkit/state.yaml`):

```yaml
workspace_id: "<uuid>"

destinations:
  clickhouse:
    id: "<uuid>"

definitions:
  m365:
    id: "<uuid>"
  zoom:
    id: "<uuid>"
  bamboohr:
    id: "<uuid>"

tenants:
  example-tenant:                     # matches connections/example-tenant.yaml filename
    connectors:
      m365:                           # connector name from descriptor.yaml
        m365-main:                    # source-id from K8s Secret annotation
          source_id: "<uuid>"
          connection_id: "<uuid>"
      zoom:
        zoom-main:
          source_id: "<uuid>"
          connection_id: "<uuid>"
      bamboohr:
        bamboohr-main:
          source_id: "<uuid>"
          connection_id: "<uuid>"
```

**Access paths** (all deterministic, no search):

| What | Path | Inputs |
|------|------|--------|
| Workspace | `workspace_id` | none |
| Destination | `destinations.clickhouse.id` | none |
| Definition | `definitions.{connector}.id` | connector name |
| Source | `tenants.{tenant}.connectors.{connector}.{source_id}.source_id` | tenant, connector, source_id |
| Connection | `tenants.{tenant}.connectors.{connector}.{source_id}.connection_id` | tenant, connector, source_id |
| All connections for tenant | `tenants.{tenant}.connectors` | tenant |

### 3.4 Internal Dependencies

| Dependency Module | Interface Used | Purpose |
|-------------------|----------------|----------|
| `connectors/*/descriptor.yaml` | File read | Connector name, schedule, streams config |
| `connectors/*/connector.yaml` | File read | Airbyte manifest for definition registration |
| `connections/*.yaml` | File read | Tenant config (tenant_id) |

### 3.5 External Dependencies

#### Airbyte API

| Dependency Module | Interface Used | Purpose |
|-------------------|---------------|---------|
| Airbyte Server | REST API (`/api/v1/*`) | CRUD for definitions, sources, destinations, connections |

#### Kubernetes API

| Dependency Module | Interface Used | Purpose |
|-------------------|---------------|---------|
| K8s Secrets | `kubectl get secret` | Read Airbyte auth credentials, connector credentials, ClickHouse password |
| K8s ConfigMap | `kubectl create configmap` | Persist state in-cluster |

#### ClickHouse

| Dependency Module | Interface Used | Purpose |
|-------------------|---------------|---------|
| ClickHouse | `kubectl exec clickhouse-client` | Create Bronze databases (`CREATE DATABASE IF NOT EXISTS`) |

### 3.6 Interactions & Sequences

#### Register connector definitions

**ID**: `cpt-insightspec-seq-register`

**Use cases**: `cpt-insightspec-usecase-new-connector`

**Actors**: `cpt-insightspec-actor-platform-engineer`

```
Engineer -> register.sh: register.sh m365
register.sh -> env.sh: source (resolve API, token)
register.sh -> connectors/: read connector.yaml
register.sh -> Airbyte API: POST /source_definitions/create (or update)
Airbyte API --> register.sh: definition_id
register.sh -> state.sh: write definitions.m365.id
```

#### Create connections for tenant

**ID**: `cpt-insightspec-seq-connect`

**Use cases**: `cpt-insightspec-usecase-new-connector`

**Actors**: `cpt-insightspec-actor-platform-engineer`

```
Engineer -> connect.sh: connect.sh example-tenant
connect.sh -> env.sh: source (resolve API, token)
connect.sh -> state.sh: read definitions (verify registered)
connect.sh -> K8s API: discover Secrets by label
connect.sh -> ClickHouse: CREATE DATABASE IF NOT EXISTS bronze_{connector}
connect.sh -> Airbyte API: create/update destination
connect.sh -> state.sh: write destinations.clickhouse.id
  for each connector+source_id:
    connect.sh -> Airbyte API: create/update source
    connect.sh -> Airbyte API: discover schema
    connect.sh -> Airbyte API: create/update connection
    connect.sh -> state.sh: write tenants.{tenant}.connectors.{connector}.{source_id}
```

#### Default reconcile

**ID**: `cpt-insightspec-seq-reconcile-default`

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`, `cpt-insightspec-actor-k8s-api`

Default invocation `reconcile-connectors.sh` (no subcommand). Drives Airbyte to descriptor + Secret state.

```mermaid
sequenceDiagram
    participant Op as Operator/CI
    participant R as reconcile-connectors.sh
    participant K as K8s API
    participant D as descriptor.yaml
    participant A as Airbyte API

    Op->>R: reconcile-connectors.sh [--dry-run]
    R->>K: list Secrets (label app.kubernetes.io/part-of=insight)
    K-->>R: secrets[] (with annotations connector + source-id)
    R->>D: read connectors/<name>/descriptor.yaml.version
    R->>R: build desired set (connector_name, source_id, version, sha256(secret.data))
    R->>A: source_definitions/list, sources/list, connections/list (filter by tag insight)
    A-->>R: actual resources

    loop per connector_name
        R->>R: compare descriptor.version vs definition.description
        alt mismatch
            R->>A: connector_builder_projects/update_active_manifest (description=version)
            R->>A: cascade: sources/update + connections/update (or recreate-with-state per seq-breaking-change)
        else match
            R->>R: definition no-op
        end
        R->>A: sources/update with secret.data (idempotent)
        R->>R: compute cfg_hash = sha256(secret.data)
        alt connection.tags['cfg-hash:'] != cfg_hash
            R->>A: PATCH connections/{id} tags=[insight, cfg-hash:<hash>]
        end
    end

    opt --no-gc absent
        R->>A: list resources tagged insight without matching desired entry
        R->>A: delete orphans (definitions ref_count=0, sources, connections)
    end

    R-->>Op: summary (created/updated/no-op/deleted/skipped)
```

#### Adopt one-shot

**ID**: `cpt-insightspec-seq-adopt-one-shot`

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`

Pre-migration pass that annotates existing Airbyte resources without creating, deleting, or recreating sources/connections — preserves all sync state.

```mermaid
sequenceDiagram
    participant Op as Operator
    participant R as reconcile-connectors.sh
    participant K as K8s API
    participant A as Airbyte API

    Op->>R: reconcile-connectors.sh adopt [--dry-run]
    R->>K: list Secrets (label app.kubernetes.io/part-of=insight)
    K-->>R: secrets[]
    R->>A: source_definitions/list, sources/list, connections/list

    loop per Secret
        R->>R: match Secret to existing source by name pattern
        alt source found
            R->>A: connector_builder_projects/update_active_manifest (description=descriptor.version)
            R->>A: PATCH connections/{id} tags=[insight, cfg-hash:<sha256(secret.data)>]
        else source not found
            R->>R: skip (reconcile will create later)
        end
    end

    R->>A: list duplicate definitions per connector_name with ref_count=0
    R->>A: delete duplicates only (ref_count>0 untouched)

    R-->>Op: summary (annotated/skipped/duplicates_deleted)
```

#### Breaking-change recreate with state preservation

**ID**: `cpt-insightspec-seq-breaking-change-recreate-with-state`

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`

When a connection's syncCatalog drift is breaking (changed PK or cursor field on a stream), recreate the connection while preserving Airbyte sync state via export/import.

```mermaid
sequenceDiagram
    participant R as reconcile-connectors.sh
    participant A as Airbyte API

    R->>A: connections/get (current syncCatalog)
    R->>A: sources/discover_schema (fresh)
    R->>R: detect breaking change (PK/cursor field changed on a stream)
    R->>A: POST /api/v1/state/get {connectionId}
    A-->>R: state_blob (per-stream cursors)
    R->>A: connections/delete {connectionId}
    R->>A: connections/create (new syncCatalog) → newConnectionId
    R->>A: POST /api/v1/state/create_or_update {newConnectionId, state_blob}
    R->>A: PATCH connections/{newConnectionId} tags=[insight, cfg-hash:<hash>]
    R-->>R: log: "recreated connection X→Y, state preserved"
```

#### Resolve connection by name (init-step)

- [ ] `p1` - **ID**: `cpt-insightspec-seq-resolve-connection-by-name`

**Actors**: `cpt-insightspec-actor-toolkit-cli`, `cpt-insightspec-actor-airbyte-api`

```mermaid
sequenceDiagram
  participant CW as CronWorkflow
  participant Init as airbyte-sync init-step (resolver)
  participant AB as Airbyte API
  participant Sync as airbyte-sync sync step
  CW->>Init: connection_name = "{connector}-{src}-to-clickhouse-{tenant}"
  Init->>AB: ab_list_connections
  alt name found
    AB-->>Init: connection_id (UUID)
    Init-->>Sync: outputs.connection_id
    Sync->>AB: trigger sync
  else name not found
    Init--xCW: fail("ERROR: connection name not found")
  end
```

#### Render and apply per-connector CronWorkflow

- [ ] `p1` - **ID**: `cpt-insightspec-seq-render-and-apply-cronworkflow`

**Actors**: `cpt-insightspec-actor-toolkit-cli`, `cpt-insightspec-actor-k8s-api`

```mermaid
sequenceDiagram
  participant Recon as reconcile.sh
  participant Argo as lib/argo.sh
  participant Py as render_cronworkflow.py
  participant K8s as kubectl
  Recon->>Argo: argo_apply_cronworkflow(connector, conn_name, schedule, tenant)
  Argo->>Py: stdin → render templates/cron-workflow.yaml.tpl
  Py-->>Argo: rendered YAML
  Argo->>K8s: kubectl apply -f -
  alt no diff
    K8s-->>Argo: unchanged
  else diff
    K8s-->>Argo: configured
    Argo-->>Recon: log_line INFO "applied CronWorkflow ${connector}-${tenant}-sync"
  end
```

#### Submit one-shot sync trigger on data-affecting change

- [ ] `p1` - **ID**: `cpt-insightspec-seq-sync-trigger-on-change`

**Actors**: `cpt-insightspec-actor-toolkit-cli`, `cpt-insightspec-actor-k8s-api`, `cpt-insightspec-actor-airbyte-api`

```mermaid
sequenceDiagram
  participant Recon as reconcile.sh
  participant Argo as lib/argo.sh
  participant Py as render_sync_trigger.py
  participant K8s as kubectl
  participant AB as Airbyte API (queued by Workflow init-step)
  Recon-->>Recon: detect data-affecting change (version|cfg_hash|new|recreate)
  Recon->>Argo: argo_submit_sync_trigger(connector, conn_name, tenant)
  Argo->>Py: render templates/sync-trigger.yaml.tpl
  Py-->>Argo: rendered Workflow YAML (generateName)
  Argo->>K8s: kubectl create -f -
  K8s-->>AB: airbyte-sync Workflow runs immediately
```

### 3.7 Database schemas & tables

Not applicable. The toolkit manages Airbyte resources, not database schemas. Bronze databases are created as empty databases; table creation is handled by Airbyte sync.

### 3.8 Deployment Topology

The Airbyte API is the authoritative state store; the legacy `state.yaml` file and the `airbyte-state` ConfigMap have been retired. Connection-name is the operator-facing handle (per ADR-0005); CronWorkflows reference it; the `resolve-connection-by-name` init-step in `airbyte-sync` resolves the UUID at submit time.

The toolkit runs in two execution modes — both read state from Airbyte directly:
- **Host**: `init.sh`, manual operations, CI/CD invoke `reconcile-connectors.sh` against the cluster's Airbyte API.
- **In-cluster**: cluster-level Argo CronWorkflow (per ADR-0006) and one-shot `airbyte-sync` Workflows invoke the toolbox image; auth is the same JWT path as host runs (per `cpt-insightspec-fr-jwt-auth`).

Authoritative anchors in Airbyte:
- `definition.declarativeManifest.description` — descriptor version anchor (per ADR-0001).
- `connection.tags` — membership (`insight`) + config hash (`cfg-hash:<sha>`).
- `connection.name` — the canonical operator-facing handle resolved by Argo at submit time.

### 3.9 Reconciliation Model

The reconciliation model defines the relationship between desired state (on disk + K8s) and actual state (in Airbyte). It is implemented by `cpt-insightspec-component-reconcile-engine` (orchestrator) consuming desired state from `cpt-insightspec-component-secret-discovery` (input).

**Three layers, three triggers**:

| Layer | Desired anchor | Actual anchor | Trigger to act |
|---|---|---|---|
| Definition | `descriptor.yaml.version` | `definition.declarativeManifest.description` (nocode) / `dockerImageTag` (CDK) | mismatch → republish |
| Source | K8s Secret existence + `descriptor.yaml` exists | source named `{connector}-{source_id}-{tenant}` | absent → create; present → idempotent `sources/update` |
| Connection | source exists + (catalog from `discover_schema`) | `connection.tags['cfg-hash:']` + `connection.syncCatalog` | hash mismatch → tag patch + `sources/update`; catalog non-breaking → `connections/update`; catalog breaking → recreate-with-state |

**Decision rule for "no-op"**: when all three layers' desired anchors equal their actual anchors, the engine emits `no-op` and makes no Airbyte API calls beyond list/get.

**CDK connector lifecycle** (per ADR-0011): Insight CDK connectors point at pre-built Docker images via the descriptor field `cdk_image`, which carries the full image reference (e.g. `ghcr.io/cyberfabric/source-bitbucket-cloud-insight:2026.04.21.16.10-b36cf42`). Reconcile splits this string into `dockerRepository` + `dockerImageTag` using the canonical Docker reference grammar (digest `@sha256:` first; else last `:` after last `/`; no tag → `:latest`) — no derivation, no convention, no env var. The image name has no required structure. `descriptor.version` is the Insight semantic version (audit, Argo labels) and is independent of `cdk_image`. Reconcile **never builds Docker images at runtime** — it only registers/updates the source_definition. First registration uses `source_definitions/create_custom`; subsequent image bumps (same repository, new tag) use `source_definitions/update` (existing `ab_set_definition_image_tag`). Missing `cdk_image` for `type=cdk` → reconcile WARNs and skips that connector for the run. Local-dev image build remains in `lib/cdk-build.sh` (operator-invoked, not part of the reconcile loop).

**NoCode definition lifecycle** (per ADR-0010 v2): `connectors/<area>/<name>/connector.yaml` (manifest in repo) → `connector_builder_projects/create` (builder project + draft manifest) → `connector_builder_projects/publish` (active source_definition) → on subsequent runs, `declarative_source_definitions/create_manifest` (with `setAsActiveManifest: true`) bumps the version when `descriptor.yaml.version` changes. The legacy `connector_builder_projects/update_active_manifest` endpoint was removed in Airbyte 1.7+. Orphan definitions (`custom: true`, no linked builder project) discovered during reconcile produce a WARN and are skipped — they are NOT deleted (cascade-breaks linked sources/connections per ADR-0010 §Consequences). Operators run the dedicated `tools/migrate-orphan-definition.sh <connector>` helper, which exports per-connection state, creates the new source under a temp name, recreates the connections + restores state on the new source, and only then deletes the old source as the cutover step (transactional pattern). Subsequent reconcile passes find a healthy builder + definition pair and use `create_manifest` for any further version bumps. CDK definitions retain the `create_custom` registration path. All definition iteration filters on `custom: true` per ADR-0009 to scope to Insight-managed definitions inside the shared Airbyte workspace, which is auto-discovered at runtime via `ab_workspace_id`.

Drives PRD requirements `cpt-insightspec-fr-version-driven-reconcile`, `cpt-insightspec-fr-orphan-gc`, `cpt-insightspec-fr-state-preserved-on-breaking-change`, `cpt-insightspec-fr-cli-surface`. Related ADRs are listed in §1.2 Architecture Drivers.

See sequence `cpt-insightspec-seq-reconcile-default` in §3.6 for the end-to-end flow.

### 3.10 Adoption (one-shot)

Adoption is a **migration-only** mode for clusters that pre-date this refactor. It is implemented by `cpt-insightspec-component-adopt-pass`. The intent: bring legacy Airbyte resources into the post-refactor metadata convention (description on definitions, tags on connections) **without** recreating any source or connection — sync state is preserved by construction.

**Out-of-scope for adopt**: creating new sources or connections, deleting Secret-less resources, pushing credentials via `sources/update`. Those are reconcile's responsibilities.

**Idempotent and re-runnable**: running adopt twice is a no-op on the already-annotated set. Safe to re-run after partial failures.

Drives PRD requirement `cpt-insightspec-fr-adopt-legacy-resources`. The related ADR is listed in §1.2 Architecture Drivers.

See sequence `cpt-insightspec-seq-adopt-one-shot` in §3.6 for the flow.

### 3.11 Naming Convention

The reconcile engine identifies resources by deterministic conventions, not by string parsing. Three anchors carry post-refactor semantics:

| Where | What | Format / Value | Purpose |
|---|---|---|---|
| `connectors/<name>/descriptor.yaml` | `version` field | semver-like string (baseline `2026.05.04`) | Single human-edited driver of reconcile decisions |
| Airbyte `definition.declarativeManifest.description` (nocode) | mirrors descriptor version | string equal to descriptor `version` | Marks "what version is currently published" |
| Airbyte `definition.dockerImageTag` (CDK) | mirrors descriptor version | tag including version | Marks current published image for CDK connectors |
| Airbyte `connection.tags` | membership + config hash | `["insight", "cfg-hash:<sha256(secret.data)>"]` | Membership marker + per-instance config drift detector |
| K8s Secret label | membership | `app.kubernetes.io/part-of=insight` | Discovery filter |
| K8s Secret annotations | identity | `insight.cyberfabric.com/connector=<name>`, `insight.cyberfabric.com/source-id=<id>` | Pair Secret with `connectors/<name>/descriptor.yaml` and Airbyte source name |
| Airbyte `source.name` | composed | `{connector_name}-{source_id}-{tenant_id}` | Stable lookup pattern (e.g., `bamboohr-bamboohr-main-virtuozzo`) |
| Airbyte `connection.name` | composed | `{connector_name}-{source_id}-to-clickhouse-{tenant_id}` | Stable lookup pattern (e.g., `bamboohr-bamboohr-main-to-clickhouse-virtuozzo`) |
| Airbyte `connection.namespaceFormat` | bronze database | `bronze_{connector_name_underscored}` | Per-connector ClickHouse Bronze database |

> **Tenant resolution**: `tenant_id` comes from cluster-level `ConfigMap insight-config` (data field `tenant_id`) or env var `INSIGHT_TENANT_ID` as fallback. Per-tenant `connections/<tenant>.yaml` files are removed (Decision #6).

### 3.12 Secret Validation

The secret validator (`cpt-insightspec-component-secret-validator`) is a **read-only** check run independently of reconcile. Its job: catch the most common cluster-side configuration mistakes before they become silent runtime failures.

**What it checks** (per connector that has a `secrets/connectors/*.yaml.example`):

| Check | Where | Error level |
|---|---|---|
| Required `stringData` keys present | K8s Secret | ERROR |
| Required labels present | K8s Secret | ERROR |
| Required annotations present | K8s Secret | ERROR |
| OnePasswordItem CR ↔ child Secret label/annotation parity | both | WARN (not fatal) |

The annotation-parity warning exists because the 1Password operator copies labels onto child Secrets but **not** custom annotations. Without this check, a connector can drop out of `cpt-insightspec-component-secret-discovery`'s discovery query when its CR diverges from its Secret.

**Integration**: validation is no longer a standalone CLI with top-level exit codes — `lib/validate.sh` is sourced by `reconcile-connectors/main.sh` and exposes two per-connector helpers consumed by the reconcile loop:

- `valsec_check_secret <connector> [namespace] [connector_dir]` — returns `0` when the Secret satisfies the descriptor's `secret.required_fields`, `2` when a required field is missing (the field name is printed on stdout so the caller can log it).
- `valsec_secret_missing_p <connector> [namespace]` — returns `0` when the Secret is genuinely missing (caller may cascade-delete), `1` when present, `2` on kubectl/API failure (caller MUST treat as "skip this iteration", never cascade-delete; see `cpt-insightspec-adr-credential-rotation-no-env`).

Reconcile aggregates per-connector outcomes into the loop-wide counters (`_RECONCILE_FAILED`, `_RECONCILE_SKIPPED`, `_RECONCILE_NOOP`) and surfaces them as `reconcile_run`'s exit code per DoD `cpt-insightspec-dod-reconcile-exit-code-reflects-failures`.

Drives PRD requirement `cpt-insightspec-fr-secret-validation`. The related ADR is listed in §1.2 Architecture Drivers.

`run-init.sh` runs the validator first (before reconcile/adopt) so credential issues fail fast rather than silently disabling discovery.

### 3.13 Argo Integration

The reconcile loop manages Argo objects per connector. Three components in §3.2 cooperate:

- `cpt-insightspec-component-argo-cronworkflow-renderer` — renders `templates/cron-workflow.yaml.tpl` per connector and applies via `kubectl apply -f -`. Per-connector name pattern `${connector}-${tenant}-sync`. Schedule precedence resolved by reconcile (Secret annotation > descriptor > default `0 0 * * *`). References ADR-0005, ADR-0006.
- `cpt-insightspec-component-argo-sync-trigger` — renders `templates/sync-trigger.yaml.tpl` and creates a one-shot `airbyte-sync` Workflow when reconcile detects a data-affecting change. References ADR-0008.
- `cpt-insightspec-component-argo-name-resolver` — init-step inside the `airbyte-sync` WorkflowTemplate that resolves `connection_name` → `connection_id` via `ab_list_connections` at submit time; miss fails the Workflow with `ERROR: connection name not found`. References ADR-0005.

The CronWorkflow lifecycle is driven by `cpt-insightspec-component-reconcile-engine`: render+apply on new/changed Secrets; cascade-delete the CronWorkflow named `${connector}-${tenant}-sync` when the Secret disappears (alongside connection, source, and definition with `ref_count=0`). Bronze ClickHouse data is preserved across cascade-delete.

Sequences: `cpt-insightspec-seq-resolve-connection-by-name`, `cpt-insightspec-seq-render-and-apply-cronworkflow`, `cpt-insightspec-seq-sync-trigger-on-change` (all in §3.6).

Covers: `cpt-insightspec-fr-name-based-connection-resolve`, `cpt-insightspec-fr-auto-trigger-sync-on-data-change`, `cpt-insightspec-fr-cascade-delete-cronworkflow`.

### 3.14 Cron Self-Run + Leak Guarantees

The toolkit self-runs via `cpt-insightspec-component-reconcile-cronworkflow` (defined in §3.2): a cluster-level Argo `CronWorkflow` named `insight-reconcile-loop` deployed by the umbrella chart at `charts/insight/templates/ingestion/reconcile-cron.yaml`. Schedule template: `{{ .Values.ingestion.reconcile.schedule | default "*/15 * * * *" }}`. References ADR-0006.

**Leak invariants** (verified by the Phase 18 idempotency harness running the loop 1000+ times on a quiet cluster):

- No `/tmp/airbyte-token*` or `/tmp/pf-*.log` residue across runs.
- No orphan `kubectl port-forward` background processes.
- No duplicate Airbyte sources/connections/definitions created on no-op runs.
- No duplicate Argo CronWorkflows.
- No log file growth on quiet runs (zero log lines emitted).
- No state on disk between runs (the cron pod exits per run; the next tick starts fresh).

**Bootstrap behaviour**: on a fresh cluster, the reconcile loop creates N connectors and submits N parallel sync Workflows in one tick; Airbyte itself queues these; the toolkit performs no application-level rate-limiting.

Covers: `cpt-insightspec-fr-cron-self-run`, `cpt-insightspec-fr-leak-free-loop`.

### 3.15 File Log Destination

Durable change/error history is provided by `cpt-insightspec-component-reconcile-file-logger` (defined in §3.2). References ADR-0006.

Destinations and policy (recapped here for §3.13/3.14 readers; full responsibility scope in §3.2):

- In-cluster: `/var/log/insight/reconcile-${YYYY-MM-DD}.log` on PVC `insight-reconcile-logs` — default 5Gi via Helm value `ingestion.reconcile.logs.size`; storage class via `ingestion.reconcile.logs.storageClass`.
- Local: `${XDG_STATE_HOME:-$HOME/.local/state}/insight/reconcile-${YYYY-MM-DD}.log` (append).
- Daily filename rotation only (no size-based rotation).
- Logs ONLY on changes/errors. Quiet runs emit ZERO log lines and exactly ONE stdout summary line for `kubectl logs` sanity.

Covers: `cpt-insightspec-fr-file-persistent-logs`.

## 4. Additional context

### Migration from old scripts

Old scripts to delete after toolkit is operational:

| Old script | Replaced by |
|------------|-------------|
| `scripts/airbyte-state.sh` | `airbyte-toolkit/lib/state.sh` |
| `scripts/sync-airbyte-state.sh` | `airbyte-toolkit/sync-state.sh` |
| `scripts/resolve-airbyte-env.sh` | `airbyte-toolkit/lib/env.sh` |
| `scripts/upload-manifests.sh` | `airbyte-toolkit/register.sh` |
| `scripts/apply-connections.sh` | `airbyte-toolkit/connect.sh` |

State files to delete:
- `connections/.airbyte-state.yaml`
- `connections/.state/` directory

Consumers to update:

| Consumer | Change |
|----------|--------|
| `run-sync.sh` | Read `tenants.{tenant}.connectors.{connector}.{source_id}.connection_id` from `airbyte-toolkit/state.yaml` |
| `sync-flows.sh` | Iterate `tenants.{tenant}.connectors` from `airbyte-toolkit/state.yaml` |
| `run-init.sh` | Call toolkit scripts instead of old scripts |
| `update-connectors.sh` | Call `airbyte-toolkit/register.sh` |
| `update-connections.sh` | Call `airbyte-toolkit/connect.sh` |
| `cleanup.sh` | Delete `airbyte-toolkit/state.yaml` instead of old files |
| `.gitignore` | Update paths |
| `.dockerignore` | Update paths |
| `README.md` | Update documentation |
| Connector SKILL.md | Update references |

### State library API

`lib/state.sh` exposes these functions when sourced:

| Function | Arguments | Description |
|----------|-----------|-------------|
| `state_get <path>` | Dot-separated YAML path | Returns value at path (empty string if missing) |
| `state_set <path> <value>` | Dot-separated YAML path, value | Sets value at path, creates intermediate maps |
| `state_delete <path>` | Dot-separated YAML path | Removes key at path |
| `state_list <path>` | Dot-separated YAML path to a map | Returns keys of the map |
| `state_dump` | none | Returns full state YAML |

All write operations persist to file and (if in-cluster) to ConfigMap atomically.

## 5. Traceability

- **PRD**: [PRD.md](./PRD.md)
- **ADRs**: [ADR/](./ADR/)

### PRD §5.6 (Reconcile Engine Phase 2) → DESIGN

| PRD FR | DESIGN element(s) |
|---|---|
| `cpt-insightspec-fr-cron-self-run` | `cpt-insightspec-component-reconcile-cronworkflow` (§3.14) |
| `cpt-insightspec-fr-name-based-connection-resolve` | `cpt-insightspec-component-argo-name-resolver` (§3.13), `cpt-insightspec-seq-resolve-connection-by-name` (§3.6) |
| `cpt-insightspec-fr-auto-trigger-sync-on-data-change` | `cpt-insightspec-component-argo-sync-trigger` (§3.13), `cpt-insightspec-seq-sync-trigger-on-change` (§3.6) |
| `cpt-insightspec-fr-file-persistent-logs` | `cpt-insightspec-component-reconcile-file-logger` (§3.15) |
| `cpt-insightspec-fr-cascade-delete-cronworkflow` | `cpt-insightspec-component-argo-cronworkflow-renderer` (§3.13), `cpt-insightspec-seq-render-and-apply-cronworkflow` (§3.6) |
| `cpt-insightspec-fr-leak-free-loop` | `cpt-insightspec-component-reconcile-cronworkflow` (§3.14, leak invariants) |

### Changelog

- 2026-05-05 — v1.1 — Added §3.13 (Argo Integration), §3.14 (Cron Self-Run + Leak Guarantees), §3.15 (File Log Destination), three new sequences in §3.6 (`seq-resolve-connection-by-name`, `seq-render-and-apply-cronworkflow`, `seq-sync-trigger-on-change`), and §5 traceability rows for the six Phase 1 FRs.
- 2026-05-06 — v1.1 — Added ADR-0009 / ADR-0010 rows in §1.2 architecture-drivers table; added NoCode definition lifecycle paragraph in §3.9.
- 2026-05-07 — v1.1 — Added ADR-0011 row in §1.2 architecture-drivers table; added CDK connector lifecycle paragraph in §3.9 (pre-built ghcr.io images, no runtime docker build).
- 2026-05-07 — v1.1 — Revised §1.2 ADR-0011 row + §3.9 CDK lifecycle paragraph for the `cdk_image` single-field descriptor approach (full image reference; no derivation, no `IMAGE_REGISTRY` env, no naming convention).
