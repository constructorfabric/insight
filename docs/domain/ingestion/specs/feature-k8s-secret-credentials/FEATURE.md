---
status: proposed
date: 2026-04-02
---

# Feature: K8s Secret Credential Resolution

- [ ] `p1` - **ID**: `cpt-insightspec-featstatus-k8s-secret-credentials`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Configure Connector from K8s Secret](#configure-connector-from-k8s-secret)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Merge Credentials](#merge-credentials)
  - [Discover Secrets](#discover-secrets)
- [4. States (CDSL)](#4-states-cdsl)
  - [Credential Resolution (N/A)](#credential-resolution-na)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Secret Discovery and Resolution](#secret-discovery-and-resolution)
  - [Credential Merge Logic](#credential-merge-logic)
  - [Multi-Instance Support](#multi-instance-support)
  - [Backward Compatibility](#backward-compatibility)
  - [Error Handling](#error-handling)
  - [Per-Connector Secret Documentation](#per-connector-secret-documentation)
  - [Example Tenant Config Update](#example-tenant-config-update)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

- [ ] `p2` - `cpt-insightspec-feature-k8s-secret-credentials`

### 1.1 Overview

Enable `airbyte-toolkit/connect.sh` to discover and resolve connector credentials from Kubernetes Secrets using label-based discovery, replacing inline plaintext credentials in tenant YAML files. The script discovers Secrets by label `app.kubernetes.io/part-of: insight`, reads connector type and source ID from annotations, merges Secret data with non-credential fields from tenant config, and passes the combined configuration to the Airbyte API.

### 1.2 Purpose

Consumers deploy Constructor Insight into their own K8s clusters and need to manage connector credentials through their existing secret management infrastructure (Vault + ESO, Sealed Secrets, manual). This feature provides the standard credential resolution mechanism that works uniformly across local development (kind) and production environments.

**Requirements**: `cpt-insightspec-fr-ing-secret-management`

**Principles**: `cpt-insightspec-principle-ing-tenant-isolation`

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-insightspec-actor-ing-platform-engineer` | Creates K8s Secrets with connector credentials, configures RBAC |
| `cpt-insightspec-actor-ing-workspace-admin` | Manages tenant YAML with non-credential config |
| `cpt-insightspec-actor-ing-toolbox` | Runs `airbyte-toolkit/connect.sh` inside the cluster, reads Secrets |
| `cpt-insightspec-actor-ing-airbyte` | Receives resolved credentials via API |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md)
- **Design**: [DESIGN.md](../DESIGN.md)
- **ADR**: [ADR-0003](../ADR/0003-k8s-secrets-credentials.md) `cpt-insightspec-adr-k8s-secrets-credentials`
- **Dependencies**: `cpt-insightspec-feature-terraform-connections` (connections must exist before credential resolution changes)

## 2. Actor Flows (CDSL)

### Configure Connector from K8s Secret

- [ ] `p1` - **ID**: `cpt-insightspec-flow-k8s-secret-credentials-configure`

**Actors**:
- `cpt-insightspec-actor-ing-platform-engineer`
- `cpt-insightspec-actor-ing-toolbox`

**Success Scenarios**:
- Secret exists with correct annotations and all required fields; Airbyte source created successfully
- Secret exists alongside inline config; fields merged correctly with inline taking precedence

**Error Scenarios**:
- No K8s Secret found for a connector; connector is skipped with a log message
- Secret found but missing `insight.cyberfabric.com/source-id` annotation; script exits with error
- Secret found but missing required credential fields; Airbyte API returns validation error
- kubectl not available or RBAC denies access; script exits with error

**Steps**:
1. [ ] - `p1` - Toolbox loads tenant YAML and iterates over `connectors` dict - `inst-load-tenant`
2. [ ] - `p1` - **FOR EACH** connector entry in tenant config - `inst-iterate-connectors`
   1. [ ] - `p1` - Find matching descriptor by `name` field in `descriptor.yaml` files - `inst-find-descriptor`
   2. [ ] - `p1` - Discover K8s Secrets: `kubectl get secrets -l app.kubernetes.io/part-of=insight` filtered by annotation `insight.cyberfabric.com/connector={connector_name}` - `inst-discover-secrets`
   3. [ ] - `p1` - **IF** no matching Secret found - `inst-check-secret-exists`
      1. [ ] - `p1` - Log warning: "No K8s Secret found for connector {name}, checking inline credentials" - `inst-log-no-secret`
      2. [ ] - `p1` - Log error: "No K8s Secret found for connector {name}, skipping" and skip connector - `inst-skip-no-creds`
   4. [ ] - `p1` - **ELSE** - `inst-secret-found`
      1. [ ] - `p1` - **IF** multiple Secrets match same connector name - `inst-check-multi-secret`
         1. [ ] - `p1` - Treat each Secret as a separate connector instance (multi-instance support) - `inst-multi-instance`
      2. [ ] - `p1` - **FOR EACH** matching Secret - `inst-iterate-secrets`
         1. [ ] - `p1` - Read annotation `insight.cyberfabric.com/source-id` as `insight_source_id` - `inst-read-source-id`
         2. [ ] - `p1` - Read and base64-decode all Secret `.data` fields - `inst-decode-secret`
         3. [ ] - `p1` - Algorithm: merge credentials using `cpt-insightspec-algo-k8s-secret-credentials-merge` - `inst-call-merge`
         4. [ ] - `p1` - Set `insight_tenant_id` from tenant YAML `tenant_id` - `inst-set-tenant-id`
         5. [ ] - `p1` - Set `insight_source_id` from Secret annotation - `inst-set-source-id`
         6. [ ] - `p1` - Pass merged config to Airbyte source create/update API - `inst-call-airbyte`

## 3. Processes / Business Logic (CDSL)

### Merge Credentials

- [ ] `p1` - **ID**: `cpt-insightspec-algo-k8s-secret-credentials-merge`

**Input**: Secret data dict (decoded), inline config dict (from tenant YAML), connector name

**Output**: merged credentials dict ready for Airbyte `connectionConfiguration`

**Steps**:
1. [ ] - `p1` - Start with empty result dict - `inst-init-result`
2. [ ] - `p1` - Copy all key-value pairs from decoded Secret data into result - `inst-copy-secret`
3. [ ] - `p1` - **FOR EACH** key-value in inline config - `inst-iterate-inline`
   1. [ ] - `p1` - **IF** key is a metadata field (`secretRef`, `insight_source_id`) - `inst-skip-meta`
      1. [ ] - `p1` - Skip (not an Airbyte config field) - `inst-skip-meta-action`
   2. [ ] - `p1` - **ELSE** - `inst-overlay-inline`
      1. [ ] - `p1` - Overlay into result (inline takes precedence over Secret) - `inst-overlay-action`
4. [ ] - `p1` - **RETURN** result dict - `inst-return-merged`

### Discover Secrets

- [ ] `p1` - **ID**: `cpt-insightspec-algo-k8s-secret-credentials-discover`

**Input**: namespace (from kubectl context), connector name (optional filter)

**Output**: list of Secret objects with parsed annotations

**Steps**:
1. [ ] - `p1` - Execute `kubectl get secrets -l app.kubernetes.io/part-of=insight -o json` in current namespace - `inst-kubectl-get`
2. [ ] - `p1` - **TRY** - `inst-try-kubectl`
   1. [ ] - `p1` - Parse JSON response, extract `.items[]` - `inst-parse-items`
3. [ ] - `p1` - **CATCH** kubectl error (not found, RBAC denied, timeout) - `inst-catch-kubectl`
   1. [ ] - `p1` - Log error with kubectl exit code and stderr - `inst-log-kubectl-error`
   2. [ ] - `p1` - **EXIT** with error (no silent fallback) - `inst-exit-error`
4. [ ] - `p1` - **FOR EACH** Secret in items - `inst-filter-secrets`
   1. [ ] - `p1` - Extract annotation `insight.cyberfabric.com/connector` as connector type - `inst-extract-type`
   2. [ ] - `p1` - Extract annotation `insight.cyberfabric.com/source-id` as source ID - `inst-extract-source-id`
   3. [ ] - `p1` - **IF** connector name filter provided AND type does not match - `inst-check-filter`
      1. [ ] - `p1` - Skip this Secret - `inst-skip-filtered`
5. [ ] - `p1` - **RETURN** list of matching Secrets with parsed metadata - `inst-return-secrets`

## 4. States (CDSL)

### Credential Resolution (N/A)

Not applicable. Credential resolution is a stateless process — no entity lifecycle. Secrets are read, merged, and passed to Airbyte in a single invocation with no persistent state transitions.

## 5. Definitions of Done

### Secret Discovery and Resolution

- [ ] `p1` - **ID**: `cpt-insightspec-dod-k8s-secret-credentials-discovery`

The system **MUST** discover K8s Secrets by label `app.kubernetes.io/part-of=insight` and resolve connector type from annotation `insight.cyberfabric.com/connector`.

**Implements**:
- `cpt-insightspec-flow-k8s-secret-credentials-configure`
- `cpt-insightspec-algo-k8s-secret-credentials-discover`

**Covers (PRD)**:
- `cpt-insightspec-fr-ing-secret-management`

**Touches**:
- Script: `src/ingestion/airbyte-toolkit/connect.sh`

### Credential Merge Logic

- [ ] `p1` - **ID**: `cpt-insightspec-dod-k8s-secret-credentials-merge`

The system **MUST** merge K8s Secret data with inline tenant YAML fields, where inline values take precedence over Secret values. Fields `secretRef` and `insight_source_id` in tenant YAML **MUST** be excluded from the merged config passed to Airbyte.

**Implements**:
- `cpt-insightspec-algo-k8s-secret-credentials-merge`

**Touches**:
- Script: `src/ingestion/airbyte-toolkit/connect.sh`

### Multi-Instance Support

- [ ] `p1` - **ID**: `cpt-insightspec-dod-k8s-secret-credentials-multi-instance`

The system **MUST** support multiple Secrets for the same connector type (e.g., two M365 tenants), each with a distinct `insight.cyberfabric.com/source-id` annotation. Each Secret **MUST** result in a separate Airbyte source.

**Implements**:
- `cpt-insightspec-flow-k8s-secret-credentials-configure`

**Touches**:
- Script: `src/ingestion/airbyte-toolkit/connect.sh`

### Backward Compatibility

- [ ] `p1` - **ID**: `cpt-insightspec-dod-k8s-secret-credentials-backward-compat`

The system **MUST** fall back to inline credentials from tenant YAML when no matching K8s Secret is found. Existing tenant configs without K8s Secrets **MUST** continue to work without modification.

**Implements**:
- `cpt-insightspec-flow-k8s-secret-credentials-configure`

**Touches**:
- Script: `src/ingestion/airbyte-toolkit/connect.sh`

### Error Handling

- [ ] `p1` - **ID**: `cpt-insightspec-dod-k8s-secret-credentials-errors`

The system **MUST** produce clear error messages when: (a) no K8s Secret and no inline credentials exist for a connector, (b) kubectl fails with RBAC or connectivity errors, (c) Secret is found but annotation `insight.cyberfabric.com/connector` is missing.

**Implements**:
- `cpt-insightspec-flow-k8s-secret-credentials-configure`
- `cpt-insightspec-algo-k8s-secret-credentials-discover`

**Touches**:
- Script: `src/ingestion/airbyte-toolkit/connect.sh`

### Per-Connector Secret Documentation

- [ ] `p2` - **ID**: `cpt-insightspec-dod-k8s-secret-credentials-docs`

Every connector **MUST** have a `README.md` documenting: (a) a complete K8s Secret YAML example with correct labels, annotations, and all required data fields, (b) instructions for obtaining credential values (e.g., Azure portal, Zoom marketplace), (c) which fields are required vs optional.

**Implements**:
- `cpt-insightspec-flow-k8s-secret-credentials-configure`

**Touches**:
- `src/ingestion/connectors/*/README.md` (6 connectors)

### Example Tenant Config Update

- [ ] `p2` - **ID**: `cpt-insightspec-dod-k8s-secret-credentials-example`

The example tenant config (`connections/example-tenant.yaml`) **MUST** be updated to remove inline credentials, showing only `tenant_id` and non-credential connector fields (e.g., `start_date`). A comment **MUST** reference K8s Secrets as the credential source.

**Touches**:
- `src/ingestion/connections/example-tenant.yaml`

## 6. Acceptance Criteria

- [ ] Connector configured via K8s Secret works end-to-end: Secret discovered, credentials resolved, Airbyte source created, sync runs successfully
- [ ] Multiple Secrets for the same connector type (multi-instance) create separate Airbyte sources with distinct `insight_source_id`
- [ ] Inline-only tenant config (no K8s Secrets) continues to work without changes
- [ ] Missing Secret with no inline fallback produces a clear error message naming the expected Secret label and annotation
- [ ] All 6 connectors (bamboohr, m365, zoom, cursor, claude-admin, bitbucket-server) have `README.md` with K8s Secret specification
- [ ] Example tenant config updated to reflect Secret-based credential flow
