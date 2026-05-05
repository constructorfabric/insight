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

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Requires `kubectl` with access to the cluster (for reading K8s Secrets).
- Requires `python3` (3.10+) with `pyyaml` (available in toolbox image).
- Requires `node` (for JWT minting via `crypto` module) or equivalent.
- Airbyte API must be reachable (localhost via port-forward, or in-cluster service URL).

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

### 5.3 State Synchronization

#### Sync from Airbyte API

- [ ] `p2` - **ID**: `cpt-insightspec-fr-sync-state`

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

- Airbyte API is reachable (port-forwarded on host, or in-cluster via service URL).
- K8s Secrets exist and are correctly labeled before toolkit commands run.
- One Airbyte workspace per cluster (the default workspace created by Helm chart).
- Tenant ID is currently a free-form string; will migrate to UUID. No format validation enforced now.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| State file corruption (manual edit, partial write) | Resources orphaned in Airbyte | `sync-state.sh` recovers from API |
| Airbyte API breaking changes | Toolkit commands fail | Pin Airbyte Helm chart version; test after upgrades |
| JWT secret rotation | Auth failures | Toolkit resolves token fresh on every invocation |
