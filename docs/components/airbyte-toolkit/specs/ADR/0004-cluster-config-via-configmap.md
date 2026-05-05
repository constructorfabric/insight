---
status: accepted
date: 2026-05-04
decision-makers: platform-engineering
---

# ADR-0004: Cluster Config (`tenant_id`) via Kubernetes ConfigMap


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — `connections/<tenant>.yaml` in repo](#option-a--connectionstenantyaml-in-repo)
  - [Option B — Read from `kubectl config current-context`](#option-b--read-from-kubectl-config-current-context)
  - [Option C — `ConfigMap insight-config` + env override](#option-c--configmap-insight-config--env-override)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-cluster-config-via-configmap`
## Context and Problem Statement

Pre-refactor, each cluster declared its identity via per-tenant YAML files in `src/ingestion/connections/<tenant>.yaml`. These files held only `tenant_id` and (in some cases) destination overrides — a few lines, but enough to encourage operators to commit secrets, drift across clusters, and require a per-cluster file per migration.

The new reconcile engine needs `tenant_id` in exactly one place: when composing source/connection names (`{connector}-{source-id}-{tenant_id}`) and when populating `insight_tenant_id` in `source.connectionConfiguration`. We have removed `connections/<tenant>.yaml` files (they had no other content). Where does `tenant_id` live now?

## Decision Drivers

- **Cluster-local**: `tenant_id` is a property of the cluster, not the codebase — multi-tenant deployments should differ here.
- **Discoverable from inside cluster**: the toolkit may run inside a K8s Job; it must reach the value without relying on host-side env files.
- **Operator-readable**: not a UUID-encoded environment string; should be inspectable with `kubectl get`.
- **Idempotent and version-tracked**: the value is set once at cluster setup; subsequent operator changes need an explicit step (audit trail).
- **Minimum infrastructure**: no new operators or CRDs.

## Considered Options

- **Option A** — Resurrect `connections/<tenant>.yaml` files in the repo. The toolkit reads `tenant_id` from a default-named file or a flag.
- **Option B** — Read `tenant_id` from `kubectl config current-context` (cluster name) on the host machine.
- **Option C** — Cluster-level `ConfigMap insight-config` in namespace `insight` (or `data`), with `tenant_id` as a key. The toolkit reads via `kubectl get configmap insight-config -o jsonpath='{.data.tenant_id}'`. Operator can override at run-time via env `INSIGHT_TENANT_ID`.

## Decision Outcome

Chosen option: **Option C — `ConfigMap insight-config` with env override**.

**Justification**: a ConfigMap is the K8s-native vehicle for cluster-level configuration. It is read with the same `kubectl` calls the toolkit already uses for Secrets, no new tooling. It survives operator workstation changes (lives on the cluster), is set once at provisioning, and is queryable post-hoc to confirm cluster identity. The env override `INSIGHT_TENANT_ID` covers ad-hoc invocations (CI smoke tests, integration tests) without forcing a ConfigMap edit.

### Consequences

- **Good**, because cluster identity lives on the cluster, where it logically belongs.
- **Good**, because zero new infrastructure: ConfigMaps are built into K8s.
- **Good**, because read path is uniform with K8s Secret access already used by the toolkit.
- **Good**, because operator override via `INSIGHT_TENANT_ID` makes the toolkit testable without touching the cluster.
- **Good**, because `kubectl get configmap insight-config -o yaml` provides immediate visibility of cluster identity for support tickets.
- **Bad**, because requires a one-time setup step at cluster provisioning (writing the ConfigMap). Mitigation: `run-init.sh` checks for the ConfigMap and either creates a default or fails with a clear error message.
- **Bad**, because incorrect or missing ConfigMap is a silent failure mode if the toolkit is invoked from an unconfigured cluster. Mitigation: `secret-validator` (run by `run-init.sh` first) reports missing `insight-config` ConfigMap as ERROR.

### Confirmation

- `kubectl get configmap insight-config -n insight -o jsonpath='{.data.tenant_id}'` returns the cluster's tenant ID; reconcile uses the value without falling back.
- Setting `INSIGHT_TENANT_ID=test-tenant` and invoking `reconcile-connectors.sh --dry-run` resolves the env value, ignoring any ConfigMap content (precedence test).
- Removing the ConfigMap and unsetting the env var causes `reconcile-connectors.sh` to abort with a clear message ("`tenant_id` not configured: set `INSIGHT_TENANT_ID` or create `ConfigMap insight-config`").

## Pros and Cons of the Options

### Option A — `connections/<tenant>.yaml` in repo

A YAML file per cluster (e.g., `connections/virtuozzo.yaml`) with `tenant_id: virtuozzo`. The toolkit reads from a default-named file or via `--tenant <name>`.

- Good, because `tenant_id` is version-controlled and reviewable.
- Neutral, because the file's content (only `tenant_id`) is sparse — historically also held destination overrides that are now redundant.
- Bad, because creates a cross-repo coupling: deploying a new cluster requires a repo PR before reconcile can run.
- Bad, because the repo file can drift from the cluster's actual identity (someone deploys to a cluster without committing the file).
- Bad, because forces operators to remember which file maps to which cluster.

### Option B — Read from `kubectl config current-context`

The toolkit derives `tenant_id` from the kubeconfig context name (e.g., `cyber-insight-k8s` → `cyber-insight-k8s` or normalized).

- Good, because zero configuration: any cluster has a context name.
- Neutral, because context names are operator-chosen and not always meaningful.
- Bad, because operator workstations may use different context names for the same cluster — same logical tenant could appear under multiple `tenant_id` values across operators.
- Bad, because inside a K8s Job (no kubeconfig file), there is no `current-context` to read.
- Bad, because rebuilding kubeconfig (e.g., after credential rotation) can change the context name silently, breaking reconcile identity.

### Option C — `ConfigMap insight-config` + env override

Cluster-level ConfigMap in namespace `insight` (default) holds `tenant_id` and other cluster identity fields. Env var `INSIGHT_TENANT_ID` overrides at run-time.

- Good, because cluster-native: `kubectl get configmap` is universal.
- Good, because read path identical for in-cluster and host invocations (kubeconfig points to the cluster either way).
- Good, because env override enables CI / test runs without cluster mutation.
- Good, because additional cluster-config fields can be added later without schema migration on the toolkit.
- Bad, because requires a one-time `kubectl apply` at provisioning. Mitigation: documented in `run-init.sh` and validated by `secret-validator`.

## More Information

- Recommended ConfigMap shape:
  ```yaml
  apiVersion: v1
  kind: ConfigMap
  metadata:
    name: insight-config
    namespace: insight
  data:
    tenant_id: virtuozzo
  ```
- Resolution precedence (toolkit): `INSIGHT_TENANT_ID` env var (if set and non-empty) → `ConfigMap insight-config.data.tenant_id` → abort.
- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — `tenant_id` is part of the source/connection naming convention used in version-driven reconcile.
  - `cpt-insightspec-adr-adoption-of-existing-resources` (ADR-0002) — adoption matches existing sources by name pattern that includes `tenant_id`.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses:

- `cpt-insightspec-fr-cli-surface` — `reconcile-connectors.sh` resolves `tenant_id` per this ADR before any Airbyte API call.
- `cpt-insightspec-component-secret-discovery` — owns the resolution path defined here.
- `cpt-insightspec-component-secret-validator` — reports missing `insight-config` ConfigMap as ERROR.
