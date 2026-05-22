---
cpt:
  artifact: PRD
  system: insightspec
  version: "1.1"
---

# PRD — Airbyte Toolkit

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
  - [5.1 State Management](#51-state-management)
  - [5.2 Resource Registration](#52-resource-registration)
  - [5.3 State Synchronization](#53-state-synchronization)
  - [5.4 Credential Resolution](#54-credential-resolution)
  - [5.5 Cleanup](#55-cleanup)
  - [5.6 Reconcile Engine](#56-reconcile-engine)
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

## 1. Overview

### 1.1 Purpose

Airbyte Toolkit is a unified CLI module for managing Airbyte resources (source definitions, sources, destinations, connections) and their state within the Insight ingestion pipeline.

It replaces five separate scripts (`airbyte-state.sh`, `sync-airbyte-state.sh`, `resolve-airbyte-env.sh`, `upload-manifests.sh`, `apply-connections.sh`) with a single cohesive module that uses one state file, one data format, and deterministic access paths to resource IDs.

### 1.2 Background / Problem Statement

The current ingestion stack manages Airbyte resources through independent shell scripts that evolved organically. Each script introduced its own state storage:

1. **Global state** (`connections/.airbyte-state.yaml`) — written by `sync-airbyte-state.sh` and `upload-manifests.sh` (via `airbyte-state.sh` library). Stores definitions and a flat tenant-keyed map of sources/connections.

2. **Per-tenant state** (`connections/.state/{tenant}.yaml`) — written by `apply-connections.sh`. Stores the same IDs in a different structure with concatenated keys (e.g., `bamboohr-bamboohr-main`).

These two state files use different key formats, different tenant naming conventions (`example-tenant` vs `example_tenant`), and are read by different consumers. Scripts that need a `connection_id` must search both files with prefix matching and dash-to-underscore conversion. This causes:

- **Duplicate resources** in Airbyte when scripts disagree on existing state.
- **Silent failures** when a consumer reads the wrong state file or mismatches a key.
- **Fragile string concatenation** for composite keys that breaks when connector or source names contain dashes.

### 1.3 Goals (Business Outcomes)

- Eliminate resource duplication caused by state disagreement between scripts.
- Remove all key-guessing logic (prefix match, dash/underscore conversion) from consumers.
- Provide a single source of truth for Airbyte resource IDs that all scripts read and write consistently.
- Reduce onboarding friction for platform engineers by consolidating five scripts into one module with clear commands.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Definition | Airbyte source definition — a registered connector type (e.g., `m365`, `zoom`). Global, not tenant-specific. |
| Source | An Airbyte source instance — a definition configured with credentials for a specific tenant and source-id. |
| Connection | An Airbyte connection — links a source to a destination with stream selection and sync schedule. |
| Destination | An Airbyte destination — shared ClickHouse instance. One per workspace. |
| Tenant | An Insight customer deployment identified by `tenant_id` (currently a string, will migrate to UUID). |
| Source-ID | Unique identifier for a credential set within a connector, from K8s Secret annotation `insight.cyberfabric.com/source-id`. |
| State file | Single YAML file tracking all Airbyte resource UUIDs managed by the toolkit. |

## 2. Actors

### 2.1 Human Actors

#### Platform Engineer

**ID**: `cpt-insightspec-actor-platform-engineer`

**Role**: Registers connectors, creates connections for tenants, runs syncs, and troubleshoots pipeline issues.
**Needs**: A single CLI to manage all Airbyte resources with clear, predictable commands and no hidden state conflicts.

### 2.2 System Actors

#### CI/CD Pipeline

**ID**: `cpt-insightspec-actor-ci-pipeline`

**Role**: Runs `init.sh` and toolkit commands during cluster provisioning. Must be idempotent and non-interactive.

#### Airbyte API

**ID**: `cpt-insightspec-actor-airbyte-api`

**Role**: External system that stores and manages definitions, sources, destinations, and connections. Toolkit communicates with it via REST API using JWT authentication.

#### Kubernetes API

**ID**: `cpt-insightspec-actor-k8s-api`

**Role**: Provides credential secrets (connector credentials, Airbyte auth secrets, ClickHouse credentials) via K8s Secret resources.

#### Toolkit CLI

**ID**: `cpt-insightspec-actor-toolkit-cli`

**Role**: The reconcile/adopt shell process itself, running either as the in-cluster cron pod (driven by the Argo CronWorkflow) or as a local operator invocation. Performs Secret discovery, Airbyte CRUD, CronWorkflow lifecycle, sync triggering, and file-persistent logging.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires `kubectl` with access to the cluster (for reading K8s Secrets).
- Requires `python3` (3.10+) with `pyyaml`, plus `yq` (Mike Farah's Go
  binary, NOT the Python wrapper) and `jq` for descriptor parsing. Host
  invocations preflight via `lib/host-side-prerequisites.sh::ensure_tooling`
  with platform-aware install policy:
    - **macOS** — auto-install via `brew` when available, else download.
    - **WSL** / **Windows native** (Git Bash, MSYS) — download static
      binaries into `~/.insight/bin/`.
    - **Linux native** — fail with a platform-specific install hint
      (apt/dnf/pacman/snap); operator installs and re-runs.
  The toolbox image ships all three pre-installed.
- Requires `node` (for JWT minting via `crypto` module) or equivalent.
- Airbyte API must be reachable. From host,
  `lib/host-side-prerequisites.sh::ensure_airbyte_pf` opens a port-forward
  to `airbyte-airbyte-server-svc:8001` with an EXIT trap; in-cluster runs
  hit the service URL directly.

## 4. Scope

### 4.1 In Scope

- Unified state file format with hierarchical structure.
- Registration of Airbyte source definitions from connector manifests.
- Creation and update of sources, destinations, and connections per tenant.
- State synchronization from Airbyte API (rebuild state from live data).
- JWT credential resolution for Airbyte API access.
- Cleanup of Airbyte resources using state as source of truth.
- In-cluster state persistence via K8s ConfigMap.

### 4.2 Out of Scope

- Airbyte Helm chart installation or upgrade.
- ClickHouse database creation (DDL).
- Argo Workflow / CronWorkflow management (`sync-flows.sh` remains separate).
- dbt model execution.
- Connector manifest authoring or validation.
- Airbyte job log retrieval (`logs.sh` remains separate).

## 5. Functional Requirements

### 5.1 State Management

#### Single state file

- [ ] `p1` - **ID**: `cpt-insightspec-fr-single-state`

The toolkit **MUST** use exactly one state file (`airbyte-toolkit/state.yaml`) for all Airbyte resource IDs.

**Rationale**: Eliminates the dual-state problem that causes resource duplication and key-guessing.

#### Hierarchical state structure

- [ ] `p1` - **ID**: `cpt-insightspec-fr-hierarchical-state`

The state file **MUST** use a hierarchical YAML structure where each resource ID is accessed via a deterministic path without string concatenation:

- `workspace_id` — top-level
- `destinations.{name}.id` — shared destinations
- `definitions.{connector}.id` — source definitions
- `tenants.{tenant}.connectors.{connector}.{source_id}.source_id` — sources
- `tenants.{tenant}.connectors.{connector}.{source_id}.connection_id` — connections

**Rationale**: Every consumer knows the exact path to any ID. No prefix matching, no key guessing.

#### Tenant key normalization

- [ ] `p1` - **ID**: `cpt-insightspec-fr-tenant-key`

The toolkit **MUST** use the tenant identifier as-is from the tenant config file name (e.g., `example-tenant` from `connections/example-tenant.yaml`). No automatic dash-to-underscore conversion.

**Rationale**: Single canonical form eliminates ambiguity. Tenant ID will migrate to UUID; normalization rules would become dead code.

#### Idempotent operations

- [ ] `p1` - **ID**: `cpt-insightspec-fr-idempotent`

All toolkit commands **MUST** be idempotent: running the same command twice with the same inputs **MUST** produce the same state without creating duplicate resources.

**Rationale**: Required for CI/CD reliability and safe re-runs after partial failures.

### 5.2 Resource Registration

#### Register definitions

- [ ] `p1` - **ID**: `cpt-insightspec-fr-register-definitions`

The toolkit **MUST** register connector manifests (`connector.yaml`) as Airbyte source definitions and store the resulting `definition_id` in state at `definitions.{connector}.id`.

**Rationale**: Definitions are global (not tenant-specific) and must be registered before sources can be created.

#### Create connections

- [ ] `p1` - **ID**: `cpt-insightspec-fr-create-connections`

The toolkit **MUST** create sources and connections for a given tenant by:
1. Reading credentials from K8s Secrets (discovered by label `app.kubernetes.io/part-of=insight`).
2. Creating or updating the shared ClickHouse destination.
3. Creating or updating a source per connector + source-id combination.
4. Creating or updating a connection linking source to destination with discovered schema.
5. Storing all IDs in state at the deterministic paths.

**Rationale**: This is the core operation that wires a tenant's data sources to the pipeline.

#### Connector lifecycle namespace and registration path

Insight connectors are namespaced by Airbyte workspace + `custom: true` flag (ADR-0009). NoCode connectors are registered through Airbyte's `connector_builder_projects` API rather than the legacy `create_custom` endpoint (ADR-0010); this provides UI editability and a clean update path for the version-bump algorithm. CDK (Docker-image) connectors continue to use `create_custom` via the CDK registration path.

CDK connectors carry their full Docker image reference in `descriptor.yaml.cdk_image` (e.g. `ghcr.io/cyberfabric/source-...:tag`). Reconcile uses this verbatim — no derivation, no convention, no `IMAGE_REGISTRY` env. See ADR-0011. NoCode connectors register via builder_projects (ADR-0010). Both paths converge on `custom: true` definitions inside the single Airbyte workspace, which is auto-discovered at runtime via `ab_workspace_id` (ADR-0009).

### 5.3 State Synchronization

#### Sync from Airbyte API

- [ ] `p2` - **ID**: `cpt-insightspec-fr-sync-state`

> **Status**: deprecated (see ADR-0001 / FEATURE-reconcile). Superseded by `cpt-insightspec-fr-cli-surface` — Airbyte itself is now the authoritative state store, so a "rebuild local state" command is no longer required. ID retained for backward compatibility.

The toolkit **MUST** provide a command that rebuilds the state file from the current Airbyte API state (definitions, sources, destinations, connections).

**Rationale**: Recovery mechanism when state file is lost, corrupted, or out of sync with Airbyte.

### 5.4 Credential Resolution

#### JWT authentication

- [ ] `p1` - **ID**: `cpt-insightspec-fr-jwt-auth`

The toolkit **MUST** resolve Airbyte API credentials (JWT token, workspace ID) from K8s Secrets and provide them to all API operations.

**Rationale**: All Airbyte API calls require authentication. Centralizing this avoids duplication.

### 5.5 Cleanup

#### Delete resources by state

- [ ] `p2` - **ID**: `cpt-insightspec-fr-cleanup`

> **Status**: deprecated (see ADR-0001 / FEATURE-reconcile). Superseded by `cpt-insightspec-fr-cli-surface` — cleanup is now expressed as orphan-GC inside the reconcile engine (driven by `insight` membership tag), not as a separate state-file-driven command. ID retained for backward compatibility.

The toolkit **MUST** provide a command that deletes all Airbyte resources (connections, sources, destinations) tracked in the state file and clears the state.

**Rationale**: Needed for full reset scenarios (breaking schema changes, re-provisioning).

### 5.6 Reconcile Engine

#### Version-driven reconcile

- [ ] `p1` - **ID**: `cpt-insightspec-fr-version-driven-reconcile`

The toolkit **MUST** treat each connector's `descriptor.yaml.version` field as the single source of truth for reconcile decisions: when the value mismatches `definition.declarativeManifest.description` in Airbyte (for nocode) or `dockerImageTag` (for CDK), the toolkit **MUST** republish the definition and cascade the change to dependent sources and connections; when it matches, the toolkit **MUST NOT** republish or recreate the definition.

**Rationale**: A single human-edited semver per connector eliminates state-file ambiguity and makes "no change → no work" deterministic at the definition layer. Storing the version on the Airbyte side removes the need for a parallel local state to know "what we last applied".

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`

#### Adopt legacy Airbyte resources

- [ ] `p1` - **ID**: `cpt-insightspec-fr-adopt-legacy-resources`

The toolkit **MUST** provide an `adopt` mode that, for every K8s Secret matched to an existing Airbyte source by naming convention, annotates the Airbyte side **without** creating, deleting, or recreating any source or connection: it **MUST** patch `definition.declarativeManifest.description` to the descriptor version, **MUST** patch `connection.tags` to include `insight` and `cfg-hash:<sha256(secret.data)>`, and **MUST** delete only those duplicate definitions whose reference count is zero.

**Rationale**: Existing clusters carry connections that have accumulated Airbyte sync state (cursors). A first-pass reconcile that performed creates/deletes would discard that state. The adopt mode is the safe migration path — metadata-only, idempotent, and re-runnable.

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`

#### Orphan garbage collection

- [ ] `p2` - **ID**: `cpt-insightspec-fr-orphan-gc`

The toolkit **MUST** delete Airbyte sources, connections, and definitions that carry the `insight` membership tag (or our naming convention) but have no corresponding K8s Secret + descriptor pair, **unless** invoked with `--no-gc`. The sweep **MUST** log every deletion target in dry-run mode before any state-changing call.

**Rationale**: Without GC, deleted Secrets leave stale Airbyte resources forever, polluting the workspace and confusing operators. The opt-out flag (`--no-gc`) covers controlled migrations where the operator wants to preserve resources temporarily.

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-ci-pipeline`, `cpt-insightspec-actor-airbyte-api`

#### Sync-state preservation on breaking change

- [ ] `p1` - **ID**: `cpt-insightspec-fr-state-preserved-on-breaking-change`

When a connection's catalog drifts in a way that requires recreation (changed primary key or cursor field on a stream), the toolkit **MUST** export the existing Airbyte connection state via `POST /api/v1/state/get`, delete and recreate the connection, then import the state via `POST /api/v1/state/create_or_update`. For non-breaking catalog drift, the toolkit **MUST** call `connections/update` only and **MUST NOT** delete the connection.

**Rationale**: Recreating a connection without state export discards every accumulated cursor — historical resync of all data, every time. Export/import preserves cursors across breaking schema changes; the in-place `connections/update` path covers the common case where state never leaves connectionId scope.

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`

#### Secret validation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-secret-validation`

The toolkit **MUST** provide a read-only command (`secrets/validate.sh`) that compares cluster Secrets in the `data` namespace against `secrets/connectors/*.yaml.example` schemas and reports drift between the OnePasswordItem custom resource and its child Secret (labels and annotations). The command **MUST NOT** modify any cluster object and **MUST** exit non-zero only on schema violations (missing required fields, missing labels), warnings on annotation drift.

**Rationale**: 1Password operator copies labels onto child Secrets but not custom annotations. Without an explicit drift check, a connector can silently fall out of discovery when its CR diverges from its Secret. Read-only failure modes keep the validator safe to run any time.

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-k8s-api`

#### Reconcile CLI surface

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cli-surface`

The toolkit **MUST** expose all reconcile and adopt operations through a single entrypoint `src/ingestion/reconcile-connectors.sh` accepting subcommand `adopt` or `reconcile` (default), and the flags `--dry-run`, `--connector <name>`, `--no-gc`. The entrypoint **MUST NOT** require any other script (`connect.sh`, `register.sh`, `cleanup.sh`, `sync-state.sh`, `reset-connector.sh`, `update-connectors.sh`, `update-connections.sh`) to be invoked directly by users or CI.

**Rationale**: One entrypoint with a small, predictable flag set is easier to discover, document, and automate in CI than a fan of scripts whose names overlap with their roles. Bad/unlabelled Secrets produce a per-connector WARN+skip rather than a global abort.

**Actors**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-ci-pipeline`

#### 5.6.7 Cron-driven self-run

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cron-self-run`

The toolkit **MUST** self-run the reconcile loop on a `*/15 * * * *` schedule via an in-cluster Argo CronWorkflow deployed by the umbrella chart at `charts/insight/templates/ingestion/reconcile-cron.yaml`, owning a ServiceAccount whose RBAC allows reading `secrets`, `onepassworditems`, and `configmaps`, and creating/deleting `workflows.argoproj.io` and `cronworkflows.argoproj.io`. The schedule **MUST** be overridable via Helm value `ingestion.reconcile.schedule`.

**Actor**: Cluster cron pod (`cpt-insightspec-actor-toolkit-cli`).
**Rationale**: Removes external orchestrator (Kestra) dependency; reconcile loop runs autonomously inside the cluster.
**Verification**: `helm template … | grep -A4 schedule` shows the schedule; integration test confirms a CronWorkflow object is created.

#### 5.6.8 Name-based connection resolve

- [ ] `p1` - **ID**: `cpt-insightspec-fr-name-based-connection-resolve`

The Argo `airbyte-sync` WorkflowTemplate **MUST** resolve `connection_id` from a `connection_name` parameter (pattern `{connector}-{source_id}-to-clickhouse-{tenant}`) at submit time via an init-step that calls `ab_list_connections`. A lookup miss **MUST** fail the Workflow with an explicit `ERROR: connection name not found` message. The toolkit **MUST NOT** hard-code `connection_id` (UUID) in any CronWorkflow spec.

**Actor**: Cluster cron pod, Airbyte API consumer (`cpt-insightspec-actor-toolkit-cli`).
**Rationale**: Recreate-with-state assigns a new UUID; storing names instead of UUIDs in CronWorkflow specs survives recreation.
**Verification**: deleting+recreating a connection yields a different UUID; the next scheduled CronWorkflow tick still completes.

#### 5.6.9 Auto-trigger sync on data-affecting change

- [ ] `p1` - **ID**: `cpt-insightspec-fr-auto-trigger-sync-on-data-change`

When a reconcile iteration performs a data-affecting change for a connector (descriptor.yaml.version bump, K8s Secret data change detected via `cfg_hash` mismatch, new connector/connection creation, or recreate-with-state on breaking syncCatalog drift) the toolkit **MUST** submit one one-shot `airbyte-sync` Workflow rendered from `templates/sync-trigger.yaml.tpl`. The toolkit **MUST NOT** submit a sync Workflow when the only change is a tag-only patch or a `definition.description`-only patch.

**Actor**: Cluster cron pod (`cpt-insightspec-actor-toolkit-cli`).
**Rationale**: Decouple data freshness from cron tick latency; avoid spurious syncs on cosmetic patches.
**Verification**: scenario tests for each trigger and exclusion.

#### 5.6.10 File-persistent logs

- [ ] `p2` - **ID**: `cpt-insightspec-fr-file-persistent-logs`

The toolkit **MUST** emit run logs to `/var/log/insight/reconcile-${YYYY-MM-DD}.log` on PVC `insight-reconcile-logs` when running in-cluster, and to `${XDG_STATE_HOME:-$HOME/.local/state}/insight/reconcile-${YYYY-MM-DD}.log` when running locally. The PVC default size **MUST** be 5Gi, overridable via Helm value `ingestion.reconcile.logs.size`. The toolkit **MUST** write log lines ONLY on changes or errors; runs that detect no changes **MUST** emit ZERO log lines to the file. Every run **MUST** emit exactly one stdout summary line for `kubectl logs` sanity.

**Actor**: Cluster cron pod / local operator (`cpt-insightspec-actor-toolkit-cli`).
**Rationale**: Durable change history across pod restarts without log noise from idle runs.
**Verification**: 100 idle runs followed by `wc -l ${log_file}` = 0; the same run sequence emits 100 stdout summary lines.

#### 5.6.11 Cascade-delete CronWorkflow on Secret-missing

- [ ] `p1` - **ID**: `cpt-insightspec-fr-cascade-delete-cronworkflow`

When a previously-known K8s Secret for a connector is no longer present, the toolkit **MUST** cascade-delete the corresponding Airbyte connection, source, definition (if its `ref_count` reaches zero), and the per-connector Argo CronWorkflow named `${connector}-${tenant}-sync`. The toolkit **MUST NOT** delete the Bronze ClickHouse data produced by prior syncs.

**Actor**: Cluster cron pod (`cpt-insightspec-actor-toolkit-cli`).
**Rationale**: Operators remove Secrets to revoke connectors; the toolkit must complete teardown without manual cleanup.
**Verification**: scenario test creates Secret → CronWorkflow appears; deletes Secret → CronWorkflow gone within next tick; ClickHouse Bronze tables retain prior rows.

#### 5.6.12 Leak-free idempotent loop

- [ ] `p2` - **ID**: `cpt-insightspec-fr-leak-free-loop`

The cron-driven reconcile loop **MUST** be safe to run 1000+ times with zero state mutation when no Secrets, descriptors, or Airbyte resources have changed. Specifically: no `/tmp/airbyte-token*` or `/tmp/pf-*.log` residue across runs, no orphan `kubectl port-forward` background processes, no duplicate Airbyte sources/connections/definitions, no duplicate Argo CronWorkflows, no growth of the daily log file.

**Actor**: Cluster cron pod / local operator (`cpt-insightspec-actor-toolkit-cli`).
**Rationale**: A cron-driven loop is run thousands of times per quarter; any per-run resource leak compounds into operational pain.
**Verification**: idempotency harness (Phase 18) runs the loop 100× and asserts the four invariants.

#### 5.6.13 Semver version format

- [ ] `p1` - **ID**: `cpt-insightspec-fr-semver-version-format`

The toolkit **MUST** accept `descriptor.yaml.version` only in strict semver `MAJOR.MINOR.PATCH` form (regex `^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$`) whenever the value differs from what Airbyte holds. Legacy non-semver values (e.g. `2026.05.04`, `1.0`) **MAY** remain in descriptors that have not yet been edited; reconcile **MUST** treat such legacy values on the Airbyte side as a `migration` bump (no full-refresh) when the operator updates the descriptor to a semver value. A non-semver target value **MUST** be rejected with a clear error that names the offending value and references ADR-0015.

**Actor**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`
**Rationale**: A parseable version format is the only way to dispatch the `major`-bump full-refresh behavior (§5.6.15) deterministically.

#### 5.6.14 Catalog refresh on every version bump

- [ ] `p1` - **ID**: `cpt-insightspec-fr-catalog-refresh-on-bump`

On every version-bump-driven definition republish (any non-`none` `bump_kind`, including `migration`), the toolkit **MUST** call `sources/discover_schema` and PATCH the existing connection's `sync_catalog` so that every stream the source now advertises is `selected: true` and every field in each stream's `jsonSchema.properties` is implicitly selected (`fieldSelectionEnabled: false`, no exclusion list). The behavior **MUST** apply identically to both nocode and cdk connectors. Re-discover **MUST NOT** run on `bump_kind == none` (string-equal target and current).

**Actor**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-airbyte-api`
**Rationale**: Without a refresh on bump, new streams or new fields added by the connector author never reach bronze on existing connections.

#### 5.6.15 Full-refresh dispatch on major bump

- [ ] `p1` - **ID**: `cpt-insightspec-fr-full-refresh-on-major-bump`

When a nocode connector's `descriptor.version` bumps such that `target.major > current.major`, the toolkit **MUST** set `dbt_full_refresh=true` on the auto-triggered one-shot sync workflow's `ingestion-pipeline` submission. On any other `bump_kind` (`none`, `patch`, `minor`, `migration`) the flag **MUST** be `false`. The flag is one-shot: it **MUST NOT** be persisted on the connection, K8s resource, or descriptor — the next scheduled CronWorkflow tick **MUST** run incremental dbt as usual.

**Actor**: `cpt-insightspec-actor-platform-engineer`
**Rationale**: A major bump signals breaking semantics; downstream silver/gold models must be re-materialized from a clean slate. Scoping the flag to the auto-trigger workflow keeps the behavior one-shot and predictable.

#### 5.6.16 No cross-connector cascade

- [ ] `p1` - **ID**: `cpt-insightspec-fr-no-cross-connector-cascade`

A major-bump full-refresh on connector A **MUST NOT** cause any Airbyte API call to be issued for connector B's source, connection, or definition — regardless of whether downstream silver/gold dbt models join A and B. Reconcile's blast radius for a major bump is exactly `descriptor.dbt_select` of the bumped connector.

**Actor**: `cpt-insightspec-actor-platform-engineer`
**Rationale**: Bronze is append-only and silver dedups via `unique_key`; cross-source consistency holds without resyncing B. Cascading would multiply rate-limit and load on unrelated sources for no data benefit.

#### 5.6.17 Descriptor `images:` block as sole source of truth for connector images

- [ ] `p1` - **ID**: `cpt-insightspec-fr-descriptor-images-block`

Every connector directory that ships at least one `Dockerfile` **MUST** declare each such image under a map-style `images:` block in its `descriptor.yaml`. The block is a YAML map keyed by free-form `<key>` strings (e.g. `cdk`, `enrich`, `bootstrap`); each entry has `name` (GHCR image short name, no registry prefix or tag), `dockerfile` (path relative to connector dir, with leading `./`), `context` (path relative to connector dir, with leading `./`), and `image` (full registry/repo:tag, or empty string `""` for not-yet-published images).

**Reserved keys with runtime semantics:**
- `cdk` — reconcile reads `images.cdk.image` to determine the CDK source image when registering an Airbyte source definition; an empty value fails loud.
- `enrich` — the enrich workflow runner reads `images.enrich.image` at workflow submission time, via reconcile passing it as a parameter. The image reference is re-read from the descriptor on EVERY submission; no Helm-time bake.

The descriptor **MUST NOT** carry top-level `cdk_image:` or `enrich_image:` fields. CI **MUST** consume the `images:` block via dynamic discovery (scan all `descriptor.yaml` files, build a matrix of `(connector_dir, key, name, dockerfile, context)`, fan-out one build per entry). After a successful image push, CI **MUST** (a) patch `images.<key>.image` in the descriptor with the new full ref AND (b) bump `descriptor.version` by one minor increment (X.Y.Z → X.(Y+1).0; strict semver, fail loud on non-semver values) per affected connector — both edits land in the same commit (no `[skip ci]`), triggering the toolbox rebuild and chart publication on the next workflow run. The minor bump makes reconcile classify the diff as `bump_kind: minor` (per ADR-0015 §5.6.14): catalog re-discovery runs on the next deploy without dispatching `dbt --full-refresh`. A descriptor with N image entries gets exactly ONE version bump per CI run, not N.

The Helm chart **MUST NOT** carry per-connector image values (e.g. `ingestion.<connector>EnrichImage`); the connector image refs travel inside the toolbox image (descriptor baked at toolbox build time).

**Actor**: `cpt-insightspec-actor-platform-engineer`, `cpt-insightspec-actor-ci-pipeline`
**Rationale**: One file is the answer to "what images does this connector ship, where are their Dockerfiles, which tag is deployed". Adding a new image kind is a descriptor edit; no new top-level field, no new ADR, no Helm wiring per kind. CI has shared build logic — no per-image-job copypaste.

#### Changelog

- 2026-05-05 — v1.1 — Added §5.6.7…§5.6.12 (cron-self-run, name-based-connection-resolve, auto-trigger-sync-on-data-change, file-persistent-logs, cascade-delete-cronworkflow, leak-free-loop) for Phase 2 of the reconcile refactor.
- 2026-05-06 — v1.1 — Added §5.2 connector-lifecycle namespace + nocode registration path (per ADR-0009 / ADR-0010).
- 2026-05-07 — v1.1 — Extended §5.2 connector-lifecycle paragraph with CDK pre-built ghcr images path (per ADR-0011 — now SUPERSEDED by ADR-0016).
- 2026-05-13 — v1.2 — Added §5.6.13…§5.6.16 (semver-version-format, catalog-refresh-on-bump, full-refresh-on-major-bump, no-cross-connector-cascade) per ADR-0015. Added §5.6.17 enrich-image-from-descriptor per ADR-0014 (both ADR-0014 and §5.6.17 in this form now SUPERSEDED by ADR-0016 / new §5.6.17 below).
- 2026-05-21 — v1.3 — Replaced §5.6.17 (enrich-image-from-descriptor) and §5.6.18 (additive descriptor-images-block) with a single §5.6.17 declaring the map-style `images:` block as the sole source of truth for connector image identity, per ADR-0016. Top-level `cdk_image:` / `enrich_image:` fields removed from all descriptors; ADR-0011 and ADR-0014 marked SUPERSEDED.

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Host and in-cluster execution

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-dual-runtime`

The toolkit **MUST** work both from the host machine (via kubectl + port-forward) and from inside a K8s pod (via service account + in-cluster API URLs).

**Threshold**: Same commands, same state format, auto-detected runtime.

**Rationale**: `init.sh` runs from host; future automation may run in-cluster.

### 6.2 NFR Exclusions

- Performance SLAs: Toolkit runs during provisioning, not in hot path. No latency requirements.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### CLI commands

- [ ] `p1` - **ID**: `cpt-insightspec-interface-toolkit-cli`

> **Status**: superseded by `cpt-insightspec-fr-cli-surface` (single `reconcile-connectors.sh` entrypoint per ADR-0001 / FEATURE-reconcile). The legacy `register.sh` / `connect.sh` / `sync-state.sh` / `cleanup.sh` scripts have been removed. ID retained for backward compatibility.

**Type**: Shell scripts (bash)

**Stability**: unstable (active development)

**Description**: Commands exposed by the toolkit module:

| Command | Description |
|---------|-------------|
| `register.sh [--all \| connector]` | Register source definitions |
| `connect.sh [--all \| tenant]` | Create sources + connections for tenant |
| `sync-state.sh` | Rebuild state from Airbyte API |
| `cleanup.sh [--all \| tenant]` | Delete resources and clear state |
| `resolve-env.sh` | Source to set `AIRBYTE_API`, `AIRBYTE_TOKEN`, `WORKSPACE_ID` |

#### State file format

- [ ] `p1` - **ID**: `cpt-insightspec-interface-state-format`

**Type**: YAML data format

**Stability**: unstable

**Description**: Consumers (e.g., `run-sync.sh`, `sync-flows.sh`) read state at well-known paths. The format is the contract between toolkit and consumers.

### 7.2 External Integration Contracts

#### Airbyte REST API

- [ ] `p1` - **ID**: `cpt-insightspec-contract-airbyte-api`

**Direction**: required from client

**Protocol/Format**: HTTP/REST with JWT Bearer authentication. Endpoints: `/api/v1/source_definitions/*`, `/api/v1/sources/*`, `/api/v1/connections/*`, `/api/v1/destinations/*`.

**Compatibility**: Tied to Airbyte server version deployed via Helm chart. No forward-compatibility guarantee.

## 8. Use Cases

#### Register and connect a new connector

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-new-connector`

**Actor**: `cpt-insightspec-actor-platform-engineer`

**Preconditions**:
- Connector manifest exists in `connectors/{category}/{name}/connector.yaml`.
- K8s Secret with credentials exists in namespace `data`.
- Tenant config exists in `connections/{tenant}.yaml`.

**Main Flow**:
1. Engineer runs `register.sh {connector}` — definition created, ID saved to state.
2. Engineer runs `connect.sh {tenant}` — source and connection created, IDs saved to state.
3. Engineer runs `sync-flows.sh {tenant}` — CronWorkflow created using connection_id from state.

**Postconditions**:
- State file contains definition_id, source_id, connection_id at deterministic paths.
- Airbyte has matching resources.

#### Recover lost state

- [ ] `p2` - **ID**: `cpt-insightspec-usecase-recover-state`

**Actor**: `cpt-insightspec-actor-platform-engineer`

**Preconditions**:
- State file is missing or corrupted.
- Airbyte has existing resources.

**Main Flow**:
1. Engineer runs `sync-state.sh` — toolkit queries Airbyte API and rebuilds state.

**Postconditions**:
- State file reflects current Airbyte resources.

## 9. Acceptance Criteria

- [ ] All consumers (`run-sync.sh`, `sync-flows.sh`, `init.sh`) read from one state file.
- [ ] No script performs prefix matching or dash/underscore conversion on state keys.
- [ ] Running `connect.sh` twice for the same tenant does not create duplicate resources.
- [ ] State file is human-readable and each ID is reachable via a documented deterministic path.
- [ ] Old scripts (`airbyte-state.sh`, `sync-airbyte-state.sh`, `upload-manifests.sh`, `apply-connections.sh`, `resolve-airbyte-env.sh`) are deleted.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Airbyte API | REST API for resource management | p1 |
| Kubernetes API | Secret reading for credentials | p1 |
| ClickHouse | Destination target (toolkit creates Airbyte destination pointing to it) | p1 |
| `pyyaml` | YAML parsing for state file | p1 |
| `node` or `python3` | JWT minting for Airbyte auth | p1 |

## 11. Assumptions

- Airbyte API is reachable (auto-port-forwarded on host via
  `lib/host-side-prerequisites.sh`, or in-cluster via service URL).
- K8s Secrets exist and are correctly labeled before toolkit commands run.
- One Airbyte workspace per cluster (the default workspace created by Helm chart).
- Tenant ID is currently a free-form string; will migrate to UUID. No format validation enforced now.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| State file corruption (manual edit, partial write) | Resources orphaned in Airbyte | `sync-state.sh` recovers from API |
| Airbyte API breaking changes | Toolkit commands fail | Pin Airbyte Helm chart version; test after upgrades |
| JWT secret rotation | Auth failures | Toolkit resolves token fresh on every invocation |
