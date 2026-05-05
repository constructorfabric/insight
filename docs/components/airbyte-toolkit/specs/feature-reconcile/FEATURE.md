# Feature: Reconcile


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Run Reconcile](#run-reconcile)
  - [Run Adopt](#run-adopt)
  - [Dry Run](#dry-run)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Discover Secrets](#discover-secrets)
  - [Compute Config Hash](#compute-config-hash)
  - [Diff Definition Version](#diff-definition-version)
  - [Diff Source Config](#diff-source-config)
  - [Diff Connection Tags](#diff-connection-tags)
  - [Garbage Collect Orphans](#garbage-collect-orphans)
  - [Export-Import State on Recreate](#export-import-state-on-recreate)
  - [Validate Secrets](#validate-secrets)
- [4. States (CDSL)](#4-states-cdsl)
  - [Connector Lifecycle State Machine](#connector-lifecycle-state-machine)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Version Bump Applied](#version-bump-applied)
  - [Config Drift Detected and Applied](#config-drift-detected-and-applied)
  - [Adoption Idempotent](#adoption-idempotent)
  - [GC Protected by --no-gc Flag](#gc-protected-by---no-gc-flag)
  - [State Preserved on Breaking Change](#state-preserved-on-breaking-change)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-insightspec-featstatus-reconcile`
## 1. Feature Context

### 1.1 Overview

The reconcile feature drives Airbyte resources (definitions, sources, connections) into the desired state declared by `connectors/*/descriptor.yaml` + K8s Secrets, idempotently and without losing accumulated sync state.

### 1.2 Purpose

This feature implements the operator-facing CLI (`reconcile-connectors.sh`) that supersedes the legacy fan of scripts (`connect.sh`, `register.sh`, `cleanup.sh`, `sync-state.sh`, `reset-connector.sh`, `update-connectors.sh`, `update-connections.sh`). One entrypoint, deterministic outcomes, no silent state drift.

**Requirements**: `cpt-insightspec-fr-version-driven-reconcile`, `cpt-insightspec-fr-adopt-legacy-resources`, `cpt-insightspec-fr-orphan-gc`, `cpt-insightspec-fr-state-preserved-on-breaking-change`, `cpt-insightspec-fr-secret-validation`, `cpt-insightspec-fr-cli-surface`

**Principles**: `cpt-insightspec-adr-version-driven-reconcile`, `cpt-insightspec-adr-adoption-of-existing-resources`, `cpt-insightspec-adr-credential-rotation-no-env`, `cpt-insightspec-adr-cluster-config-via-configmap`

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-insightspec-actor-platform-engineer` | Invokes `reconcile-connectors.sh`, reviews dry-run output, edits `descriptor.yaml.version` |
| `cpt-insightspec-actor-ci-pipeline` | Runs `reconcile-connectors.sh adopt && reconcile-connectors.sh` from `run-init.sh` during cluster bootstrap and as pre-sync step |
| `cpt-insightspec-actor-airbyte-api` | Receives definition/source/connection patches from the reconcile engine |
| `cpt-insightspec-actor-k8s-api` | Provides K8s Secrets and `ConfigMap insight-config` (tenant_id) |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md)
- **Design**: [DESIGN.md](../DESIGN.md)
- **ADR-0001**: [Version-Driven Reconcile](../ADR/0001-version-driven-reconcile.md)
- **ADR-0002**: [Adoption of Existing Resources](../ADR/0002-adoption-of-existing-resources.md)
- **ADR-0003**: [Credential Rotation No Env](../ADR/0003-credential-rotation-no-env.md)
- **ADR-0004**: [Cluster Config via ConfigMap](../ADR/0004-cluster-config-via-configmap.md)
- **Dependencies**: None (this feature is the new entrypoint; replaces legacy)

## 2. Actor Flows (CDSL)

### Run Reconcile

- [ ] `p1` - **ID**: `cpt-insightspec-flow-reconcile-run-reconcile`

**Actor**: `cpt-insightspec-actor-platform-engineer`

**Success Scenarios**:
- All connectors are in sync — engine emits `no-op` per connector, exits 0.
- One or more connectors drifted — engine applies the minimal API calls (`update_active_manifest` / `sources/update` / `PATCH connections/{id}` / state-preserved recreate) and exits 0.
- Orphan resources detected and `--no-gc` not set — engine deletes them and exits 0.

**Error Scenarios**:
- `tenant_id` not resolvable (no `INSIGHT_TENANT_ID`, no `ConfigMap insight-config`) — abort with clear error before any API call.
- Airbyte API unreachable — abort with retry-friendly error.
- Bad/unlabelled Secret — WARN + skip that connector, continue with others.

**Steps**:
1. [ ] - `p1` - Resolve tenant_id from `INSIGHT_TENANT_ID` env or `ConfigMap insight-config` - `inst-rr-resolve-tenant`
2. [ ] - `p1` - Resolve Airbyte API endpoint and JWT token via env-resolver lib - `inst-rr-resolve-airbyte-env`
3. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-discover-secrets` to build desired state - `inst-rr-discover`
4. [ ] - `p1` - API: GET source_definitions/list, sources/list, connections/list (filter `tagIds=insight`) - `inst-rr-list-actual`
5. [ ] - `p1` - **FOR EACH** connector_name in desired_state.connectors - `inst-rr-loop`
   1. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-diff-definition-version` - `inst-rr-diff-def`
   2. [ ] - `p1` - **IF** definition_diff.action == `republish` - `inst-rr-if-republish`
      1. [ ] - `p1` - API: connector_builder_projects/update_active_manifest (description=descriptor.version) - `inst-rr-republish-call`
   3. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-diff-source-config` - `inst-rr-diff-src`
   4. [ ] - `p1` - **IF** source_diff.action == `update` - `inst-rr-if-src-update`
      1. [ ] - `p1` - API: PUT /api/public/v1/sources/{id} (configuration=secret.data) - `inst-rr-src-update-call`
   5. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-diff-connection-tags` - `inst-rr-diff-tags`
   6. [ ] - `p1` - **IF** tag_diff.action == `patch_tags` - `inst-rr-if-tags-patch`
      1. [ ] - `p1` - API: PATCH /api/public/v1/connections/{id} (tags=[insight, cfg-hash:&lt;hash&gt;]) - `inst-rr-tags-patch-call`
   7. [ ] - `p1` - **IF** catalog drift is breaking - `inst-rr-if-breaking`
      1. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-export-import-state-on-recreate` - `inst-rr-recreate-with-state`
6. [ ] - `p1` - **IF** flag `--no-gc` not set - `inst-rr-if-gc`
   1. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-gc-orphans` - `inst-rr-gc-call`
7. [ ] - `p1` - **RETURN** summary (per-connector outcome: created/updated/no-op/recreated/deleted/skipped) - `inst-rr-return`

### Run Adopt

- [ ] `p1` - **ID**: `cpt-insightspec-flow-reconcile-run-adopt`

**Actor**: `cpt-insightspec-actor-platform-engineer`

**Success Scenarios**:
- Existing legacy resources present — engine annotates definition.description and connection.tags, exits 0 with `adopted_count > 0`.
- Re-run on already-adopted set — engine emits `no-op` per resource, exits 0.

**Error Scenarios**:
- Bad/unlabelled Secret — WARN + skip that connector.
- Existing source not matchable to any Secret — WARN + skip (orphan_candidate, not deleted).

**Steps**:
1. [ ] - `p1` - Resolve tenant_id (same as run-reconcile) - `inst-ad-resolve-tenant`
2. [ ] - `p1` - Resolve Airbyte env (same as run-reconcile) - `inst-ad-resolve-env`
3. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-discover-secrets` - `inst-ad-discover`
4. [ ] - `p1` - API: GET source_definitions/list, sources/list, connections/list - `inst-ad-list-actual`
5. [ ] - `p1` - **FOR EACH** secret in desired_state.secrets - `inst-ad-loop`
   1. [ ] - `p1` - Match secret → existing source by name pattern `{connector}-{source-id}-{tenant_id}` - `inst-ad-match`
   2. [ ] - `p1` - **IF** matched - `inst-ad-if-matched`
      1. [ ] - `p1` - API: connector_builder_projects/update_active_manifest (description=descriptor.version) - `inst-ad-anno-def`
      2. [ ] - `p1` - API: PATCH /api/public/v1/connections/{id} (tags=[insight, cfg-hash:&lt;hash&gt;]) - `inst-ad-anno-conn`
   3. [ ] - `p1` - **ELSE** - `inst-ad-else-skip`
      1. [ ] - `p1` - WARN: source not found, skip (reconcile mode will create it later) - `inst-ad-skip`
6. [ ] - `p1` - **FOR EACH** connector_name with multiple definitions - `inst-ad-dedup-loop`
   1. [ ] - `p1` - Compute ref_count per definition_id - `inst-ad-ref-count`
   2. [ ] - `p1` - **IF** ref_count == 0 - `inst-ad-if-orphan-def`
      1. [ ] - `p1` - API: DELETE /source_definitions/delete - `inst-ad-delete-def`
7. [ ] - `p1` - **RETURN** summary (adopted_count, skipped_count, duplicates_deleted) - `inst-ad-return`

### Dry Run

- [ ] `p2` - **ID**: `cpt-insightspec-flow-reconcile-dry-run`

**Actor**: `cpt-insightspec-actor-platform-engineer`

**Success Scenarios**:
- Operator runs `--dry-run` flag — engine prints diff report and exits 0 without touching Airbyte.

**Error Scenarios**:
- Same env/secret error scenarios as run-reconcile — abort early with clear error.

**Steps**:
1. [ ] - `p2` - Set `dry_run = true` in execution context - `inst-dr-set-flag`
2. [ ] - `p2` - **CALL** the appropriate flow (`run-reconcile` or `run-adopt` per subcommand) - `inst-dr-call-flow`
3. [ ] - `p2` - Engine intercepts every state-changing API call (connector_builder_projects/update_active_manifest, sources/update, PATCH connections, DELETE) and emits `would_call: <method> <path>` instead - `inst-dr-intercept`
4. [ ] - `p2` - Read-only API calls (list/get) execute normally - `inst-dr-readonly`
5. [ ] - `p2` - **RETURN** diff report with planned actions per connector - `inst-dr-return`

## 3. Processes / Business Logic (CDSL)

### Discover Secrets

- [ ] `p1` - **ID**: `cpt-insightspec-algo-reconcile-discover-secrets`

**Input**: kubeconfig context (current cluster), `connectors/` directory path
**Output**: `desired_state` map keyed by `(connector_name, source_id)` with fields `{secret_data, descriptor_version, cfg_hash, tenant_id}`

**Steps**:
1. [ ] - `p1` - Resolve tenant_id (env `INSIGHT_TENANT_ID` first; else `ConfigMap insight-config.data.tenant_id`; else abort) - `inst-ds-tenant`
2. [ ] - `p1` - API: kubectl get secrets -n data -l app.kubernetes.io/part-of=insight - `inst-ds-list-secrets`
3. [ ] - `p1` - **FOR EACH** secret in result - `inst-ds-loop`
   1. [ ] - `p1` - Read annotations `insight.cyberfabric.com/connector` and `insight.cyberfabric.com/source-id` - `inst-ds-anno`
   2. [ ] - `p1` - **IF** either annotation missing - `inst-ds-if-bad`
      1. [ ] - `p1` - WARN: secret unlabelled, skip - `inst-ds-warn-skip`
      2. [ ] - `p1` - CONTINUE - `inst-ds-continue`
   3. [ ] - `p1` - Read `connectors/<connector>/descriptor.yaml.version` - `inst-ds-descriptor`
   4. [ ] - `p1` - **IF** descriptor.yaml absent or version missing - `inst-ds-if-missing-desc`
      1. [ ] - `p1` - WARN: descriptor or version missing, skip - `inst-ds-warn-no-desc`
      2. [ ] - `p1` - CONTINUE - `inst-ds-continue-2`
   5. [ ] - `p1` - **CALL** `cpt-insightspec-algo-reconcile-compute-cfg-hash` (secret.data) → cfg_hash - `inst-ds-hash`
   6. [ ] - `p1` - Append `(connector, source_id) → {secret_data, descriptor_version, cfg_hash, tenant_id}` to desired_state - `inst-ds-append`
4. [ ] - `p1` - **RETURN** desired_state - `inst-ds-return`

### Compute Config Hash

- [ ] `p1` - **ID**: `cpt-insightspec-algo-reconcile-compute-cfg-hash`

**Input**: `secret.data` (base64-encoded values from K8s Secret)
**Output**: hex string `cfg_hash`

**Steps**:
1. [ ] - `p1` - Base64-decode every value in `secret.data` - `inst-cch-decode`
2. [ ] - `p1` - Build canonical JSON: keys sorted lexicographically, values verbatim, no whitespace - `inst-cch-canonical`
3. [ ] - `p1` - Compute sha256(canonical_json) → hex digest - `inst-cch-sha256`
4. [ ] - `p1` - **RETURN** hex digest - `inst-cch-return`

### Diff Definition Version

- [ ] `p1` - **ID**: `cpt-insightspec-algo-reconcile-diff-definition-version`

**Input**: connector_name, descriptor_version, list of Airbyte source_definitions (filtered by name)
**Output**: `{action: republish|noop, definition_id?}`

**Steps**:
1. [ ] - `p1` - **IF** no definition with name == connector_name - `inst-ddv-if-none`
   1. [ ] - `p1` - **RETURN** `{action: "republish", definition_id: null}` - `inst-ddv-return-publish`
2. [ ] - `p1` - **IF** multiple definitions with name == connector_name - `inst-ddv-if-multi`
   1. [ ] - `p1` - Pick the one referenced by an existing source (others are duplicates handled by adopt) - `inst-ddv-pick-active`
3. [ ] - `p1` - **IF** active_definition.declarativeManifest.description != descriptor_version - `inst-ddv-if-mismatch`
   1. [ ] - `p1` - **RETURN** `{action: "republish", definition_id: active_definition.id}` - `inst-ddv-return-mismatch`
4. [ ] - `p1` - **RETURN** `{action: "noop", definition_id: active_definition.id}` - `inst-ddv-return-noop`

### Diff Source Config

- [ ] `p1` - **ID**: `cpt-insightspec-algo-reconcile-diff-source-config`

**Input**: source name pattern, secret_data, definition_id, list of Airbyte sources
**Output**: `{action: create|update|noop, source_id?}`

**Steps**:
1. [ ] - `p1` - Compute expected_name = `{connector}-{source_id}-{tenant_id}` - `inst-dsc-name`
2. [ ] - `p1` - **IF** no source with name == expected_name - `inst-dsc-if-none`
   1. [ ] - `p1` - **RETURN** `{action: "create"}` - `inst-dsc-return-create`
3. [ ] - `p1` - **IF** existing source.sourceDefinitionId != current definition_id - `inst-dsc-if-stale-def`
   1. [ ] - `p1` - **RETURN** `{action: "update", source_id: existing.id}` (sources/update reassigns definition without recreate) - `inst-dsc-return-redirect`
4. [ ] - `p1` - **RETURN** `{action: "update", source_id: existing.id}` (idempotent — sources/update is cheap and authoritative) - `inst-dsc-return-update`

### Diff Connection Tags

- [ ] `p2` - **ID**: `cpt-insightspec-algo-reconcile-diff-connection-tags`

**Input**: connection (existing or null), expected `cfg_hash`
**Output**: `{action: create|patch_tags|noop, connection_id?}`

**Steps**:
1. [ ] - `p2` - **IF** connection is null - `inst-dct-if-no-conn`
   1. [ ] - `p2` - **RETURN** `{action: "create"}` - `inst-dct-return-create`
2. [ ] - `p2` - Find tag with prefix `cfg-hash:` in connection.tags - `inst-dct-find-tag`
3. [ ] - `p2` - **IF** tag missing OR tag.value != expected `cfg_hash` OR `insight` membership tag missing - `inst-dct-if-drift`
   1. [ ] - `p2` - **RETURN** `{action: "patch_tags", connection_id: connection.id}` - `inst-dct-return-patch`
4. [ ] - `p2` - **RETURN** `{action: "noop", connection_id: connection.id}` - `inst-dct-return-noop`

### Garbage Collect Orphans

- [ ] `p2` - **ID**: `cpt-insightspec-algo-reconcile-gc-orphans`

**Input**: list of Airbyte resources tagged `insight`, desired_state
**Output**: `{deleted_connections: [...], deleted_sources: [...], deleted_definitions: [...]}`

**Steps**:
1. [ ] - `p2` - **FOR EACH** connection in actual where tag `insight` is present - `inst-gc-conn-loop`
   1. [ ] - `p2` - **IF** (connector_name, source_id) extracted from connection.name not in desired_state - `inst-gc-conn-orphan`
      1. [ ] - `p2` - API: DELETE /api/v1/connections/delete - `inst-gc-conn-del`
2. [ ] - `p2` - **FOR EACH** source where name matches `*-{tenant_id}` and not referenced by any kept connection - `inst-gc-src-loop`
   1. [ ] - `p2` - API: DELETE /api/v1/sources/delete - `inst-gc-src-del`
3. [ ] - `p2` - **FOR EACH** definition with `description == descriptor_version` style and ref_count == 0 - `inst-gc-def-loop`
   1. [ ] - `p2` - API: DELETE /api/v1/source_definitions/delete - `inst-gc-def-del`
4. [ ] - `p2` - **RETURN** deleted counts - `inst-gc-return`

### Export-Import State on Recreate

- [ ] `p1` - **ID**: `cpt-insightspec-algo-reconcile-export-import-state-on-recreate`

**Input**: existing connection_id, new syncCatalog
**Output**: new connection_id (with state preserved)

**Steps**:
1. [ ] - `p1` - **TRY** - `inst-eisor-try`
   1. [ ] - `p1` - API: POST /api/v1/state/get {connectionId} → state_blob - `inst-eisor-get`
2. [ ] - `p1` - **CATCH** `ApiError` - `inst-eisor-catch-get`
   1. [ ] - `p1` - WARN: state export failed; ABORT (do not delete without state safely captured) - `inst-eisor-abort`
3. [ ] - `p1` - API: DELETE /api/v1/connections/delete (old connection_id) - `inst-eisor-delete`
4. [ ] - `p1` - API: POST /api/v1/connections/create (new syncCatalog) → new_connection_id - `inst-eisor-create`
5. [ ] - `p1` - API: POST /api/v1/state/create_or_update {connectionId: new_connection_id, state: state_blob} - `inst-eisor-import`
6. [ ] - `p1` - API: PATCH /api/public/v1/connections/{new_connection_id} (tags=[insight, cfg-hash:&lt;hash&gt;]) - `inst-eisor-tag`
7. [ ] - `p1` - **RETURN** new_connection_id - `inst-eisor-return`

### Validate Secrets

- [ ] `p2` - **ID**: `cpt-insightspec-algo-reconcile-validate-secrets`

**Input**: `secrets/connectors/*.yaml.example` paths, K8s Secrets in `data` ns, OnePasswordItem CRs in `data` ns
**Output**: `{errors: [...], warnings: [...]}`

**Steps**:
1. [ ] - `p2` - **FOR EACH** example file `secrets/connectors/<connector>.yaml.example` - `inst-vs-loop`
   1. [ ] - `p2` - Parse expected `stringData` keys, expected labels, expected annotations - `inst-vs-parse`
   2. [ ] - `p2` - Find Secret `insight-<connector>-main` in `data` ns - `inst-vs-find-secret`
   3. [ ] - `p2` - **IF** Secret missing - `inst-vs-if-no-secret`
      1. [ ] - `p2` - WARN: connector not deployed - `inst-vs-warn-missing`
      2. [ ] - `p2` - CONTINUE - `inst-vs-continue`
   4. [ ] - `p2` - **IF** required `stringData` key missing in Secret - `inst-vs-if-key-missing`
      1. [ ] - `p2` - ERROR: append "missing key in <secret>" - `inst-vs-err-key`
   5. [ ] - `p2` - **IF** label `app.kubernetes.io/part-of=insight` missing - `inst-vs-if-label-missing`
      1. [ ] - `p2` - ERROR: append "missing label" - `inst-vs-err-label`
   6. [ ] - `p2` - **IF** required annotations missing on Secret - `inst-vs-if-anno-missing`
      1. [ ] - `p2` - ERROR: append "missing annotation" - `inst-vs-err-anno`
   7. [ ] - `p2` - Find OnePasswordItem CR with same name (if exists) - `inst-vs-find-cr`
   8. [ ] - `p2` - **IF** CR.labels/annotations differ from Secret.labels/annotations - `inst-vs-if-drift`
      1. [ ] - `p2` - WARN: append "OnePasswordItem CR ↔ Secret drift on <field>" - `inst-vs-warn-drift`
2. [ ] - `p2` - **RETURN** {errors, warnings} (exit 1 if errors non-empty) - `inst-vs-return`

## 4. States (CDSL)

### Connector Lifecycle State Machine

- [ ] `p2` - **ID**: `cpt-insightspec-state-reconcile-connector-lifecycle`

**States**: `ABSENT`, `DEFINITION_PRESENT`, `SOURCE_PRESENT`, `CONNECTION_TAGGED`, `IN_SYNC`, `DRIFTED`

**Initial State**: `ABSENT`

**Transitions**:
1. [ ] - `p2` - **FROM** `ABSENT` **TO** `DEFINITION_PRESENT` **WHEN** connector_builder_projects/publish or update_active_manifest succeeds - `inst-cl-t1`
2. [ ] - `p2` - **FROM** `DEFINITION_PRESENT` **TO** `SOURCE_PRESENT` **WHEN** sources/create or sources/update succeeds for matching name - `inst-cl-t2`
3. [ ] - `p2` - **FROM** `SOURCE_PRESENT` **TO** `CONNECTION_TAGGED` **WHEN** PATCH connections/{id} adds tags `insight` and `cfg-hash:<hash>` - `inst-cl-t3`
4. [ ] - `p2` - **FROM** `CONNECTION_TAGGED` **TO** `IN_SYNC` **WHEN** `cfg-hash` tag matches sha256(secret.data) AND definition.description matches descriptor_version - `inst-cl-t4`
5. [ ] - `p2` - **FROM** `IN_SYNC` **TO** `DRIFTED` **WHEN** any one of: descriptor.version changed, secret.data changed, source.sourceDefinitionId points to different definition - `inst-cl-t5`
6. [ ] - `p2` - **FROM** `DRIFTED` **TO** `IN_SYNC` **WHEN** reconcile applies the relevant patches and re-verifies hashes - `inst-cl-t6`
7. [ ] - `p2` - **FROM** any state **TO** `ABSENT` **WHEN** orphan GC deletes the connector's resources (Secret was removed) - `inst-cl-t7`

## 5. Definitions of Done

### Version Bump Applied

- [ ] `p1` - **ID**: `cpt-insightspec-dod-reconcile-version-bump-applied`

The system **MUST** propagate a `descriptor.yaml.version` change to `definition.declarativeManifest.description` (nocode) or `dockerImageTag` (CDK) in Airbyte on the next reconcile invocation, without recreating dependent sources or connections.

**Implements**:
- `cpt-insightspec-flow-reconcile-run-reconcile`
- `cpt-insightspec-algo-reconcile-diff-definition-version`

**Touches**:
- API: `POST /api/v1/connector_builder_projects/update_active_manifest`
- Entities: `definition`

### Config Drift Detected and Applied

- [ ] `p1` - **ID**: `cpt-insightspec-dod-reconcile-cfg-drift-detected-and-applied`

The system **MUST** detect a change in `secret.data` via `cfg-hash` tag mismatch and apply `sources/update` + connection-tag PATCH on the next reconcile invocation, without recreating the source or connection.

**Implements**:
- `cpt-insightspec-flow-reconcile-run-reconcile`
- `cpt-insightspec-algo-reconcile-compute-cfg-hash`
- `cpt-insightspec-algo-reconcile-diff-source-config`
- `cpt-insightspec-algo-reconcile-diff-connection-tags`

**Touches**:
- API: `PUT /api/public/v1/sources/{id}`, `PATCH /api/public/v1/connections/{id}`
- Entities: `source`, `connection`

### Adoption Idempotent

- [ ] `p1` - **ID**: `cpt-insightspec-dod-reconcile-adoption-idempotent`

The system **MUST** allow `reconcile-connectors.sh adopt` to be re-run safely: a second invocation on a fully-adopted set issues zero state-changing API calls and exits 0 with `adopted_count: 0` and `noop_count: <total>`.

**Implements**:
- `cpt-insightspec-flow-reconcile-run-adopt`

**Touches**:
- API: `connector_builder_projects/get`, `connections/list`, `tags/list`
- Entities: `definition`, `connection`

### GC Protected by --no-gc Flag

- [ ] `p1` - **ID**: `cpt-insightspec-dod-reconcile-gc-protected-by-no-gc-flag`

The system **MUST** skip the orphan-sweep step entirely when `--no-gc` is supplied, even if orphans are detected. The summary reports `gc: skipped (--no-gc set)`.

**Implements**:
- `cpt-insightspec-flow-reconcile-run-reconcile`
- `cpt-insightspec-algo-reconcile-gc-orphans`

**Touches**:
- API: none under `--no-gc`
- Entities: `connection`, `source`, `definition` (untouched)

### State Preserved on Breaking Change

- [ ] `p1` - **ID**: `cpt-insightspec-dod-reconcile-state-preserved-on-breaking-change`

The system **MUST** preserve Airbyte sync state across a connection recreate triggered by a breaking syncCatalog change: per-stream cursors valid before the recreate are valid after, verified by a follow-up sync that does NOT reset to historical zero.

**Implements**:
- `cpt-insightspec-flow-reconcile-run-reconcile`
- `cpt-insightspec-algo-reconcile-export-import-state-on-recreate`

**Touches**:
- API: `POST /api/v1/state/get`, `POST /api/v1/state/create_or_update`, `POST /api/v1/connections/create`, `POST /api/v1/connections/delete`
- Entities: `connection`, `connection_state`

## 6. Acceptance Criteria

- [ ] `reconcile-connectors.sh --dry-run` returns zero diff after a clean reconcile (idempotency check).
- [ ] `reconcile-connectors.sh adopt` is idempotent: a second invocation makes zero state-changing API calls.
- [ ] Bumping `descriptor.yaml.version` triggers exactly one `update_active_manifest` API call on the next reconcile; subsequent runs report `noop` for the definition layer.
- [ ] Rotating a K8s Secret triggers exactly one `sources/update` API call and one `PATCH connections/{id}` (cfg-hash tag) on the next reconcile; the connection's `connectionId` does not change.
- [ ] Removing a K8s Secret + running `reconcile-connectors.sh` (without `--no-gc`) deletes the orphaned connection, source, and (if `ref_count == 0`) definition.
- [ ] Removing a K8s Secret + running `reconcile-connectors.sh --no-gc` leaves the orphan resources in place; summary reports them under `would_gc_if_run_without_no_gc`.
- [ ] A breaking syncCatalog change (PK or cursor field changed on a stream) recreates the connection while preserving cursors via state export/import; the next sync does NOT re-fetch from cursor zero.
- [ ] `reconcile-connectors.sh --connector <name>` operates on only the named connector; other connectors are skipped with `--connector <name>: not in scope` notes.
- [ ] An unlabelled K8s Secret in `data` ns produces a WARN log line and is skipped, without aborting the run.
- [ ] Missing `descriptor.yaml.version` for a connector with a Secret produces a WARN log line and is skipped.
- [ ] `reconcile-connectors.sh` with neither `INSIGHT_TENANT_ID` env nor `ConfigMap insight-config` aborts before any Airbyte API call with a clear error.
