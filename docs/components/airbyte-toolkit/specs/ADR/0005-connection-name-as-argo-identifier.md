---
status: accepted
date: 2026-05-05
decision-makers: platform-engineering
---

# ADR-0005: Connection Name as Argo Identifier


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Hard-code `connection_id` (UUID) in CronWorkflow spec](#option-a--hard-code-connectionid-uuid-in-cronworkflow-spec)
  - [Option B — Resolve `connection_name → connection_id` at Workflow submit time](#option-b--resolve-connectionname--connectionid-at-workflow-submit-time)
  - [Option C — Custom Kubernetes CRD with controller](#option-c--custom-kubernetes-crd-with-controller)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-connection-name-as-argo-identifier`

## Context and Problem Statement

The toolkit reconciles K8s Secrets into Airbyte connections and runs syncs via Argo CronWorkflows on a `*/15` schedule. Per ADR-0001 (version-driven reconcile) the toolkit may "recreate-with-state" a connection on breaking syncCatalog drift — the new connection has a fresh Airbyte UUID. If the per-connector CronWorkflow spec hard-codes `connection_id` (UUID), it breaks until manually patched. The expected operational pattern is: connectors are recreated, Secrets are rotated, but the CronWorkflow per `(connector, tenant)` should keep working uninterrupted.

The connection naming pattern `{connector}-{source_id}-{tenant}-conn` is already a stable invariant in the codebase (PR #281). It is unique per `(connector, source_id, tenant)` triple — operating two GitHub instances for one tenant produces two distinct names.

How do we keep per-connector CronWorkflow specs valid across connection recreate events without re-rendering them on every reconcile tick?

## Decision Drivers

- **Recreate resilience**: a connection recreate-with-state must NOT break the next CronWorkflow tick.
- **No reconcile churn on UUID drift**: CronWorkflow specs that store UUIDs would force a re-render on every recreate, even though the operator-visible spec did not change.
- **Standard-issue indirection**: prefer existing Argo machinery over a new CRD or controller.
- **Naming-uniqueness invariant already exists**: the `{connector}-{source_id}-{tenant}-conn` pattern is unique per triple and validated by reconcile.
- **Bounded API cost**: at `*/15` cadence the per-submission lookup cost must be ≤1 extra API call.

## Considered Options

- **Option A** — Hard-code `connection_id` (UUID) in CronWorkflow spec.
- **Option B** — Resolve `connection_name → connection_id` at Workflow submit time via init-step (CHOSEN).
- **Option C** — Custom Kubernetes CRD with controller for `AirbyteConnection`.

## Decision Outcome

Chosen option: **Option B — resolve `connection_name → connection_id` at Workflow submit time**.

**Justification**: per-connector CronWorkflow stores only `connection_name`. The `airbyte-sync` WorkflowTemplate gains an init-step `resolve-connection-by-name` that calls `ab_list_connections` and emits the resolved `connection_id` as a workflow output parameter consumed by the actual sync step. Lookup miss raises an explicit error (`ERROR: connection name not found`) and fails the Workflow. The CronWorkflow spec stays stable across recreate-with-state events; reconcile does not need to detect UUID drift to keep the schedule healthy.

### Consequences

- **Good**, because a recreated Airbyte connection is picked up automatically on the next CronWorkflow tick.
- **Good**, because reconcile does no re-rendering on UUID-only drift — CronWorkflow specs do not store UUIDs.
- **Good**, because cascade-delete (ADR-related FR `cpt-insightspec-fr-cascade-delete-cronworkflow`) operates on the named CronWorkflow without UUID coordination.
- **Bad**, because every Workflow submission costs one extra Airbyte API call (acceptable: cron tick is `*/15`, Airbyte API is in-cluster, latency budget is tight only on bootstrap).
- **Bad**, because the `{connector}-{source_id}-{tenant}-conn` naming uniqueness invariant becomes a hard dependency — if a sibling connector with a colliding name appears, the resolver MUST fail loudly.

### Confirmation

- Integration test: apply Secret → reconcile → CronWorkflow exists with stored `connection_name`. Recreate the connection (new UUID via `state-preserved-on-breaking-change`). Wait for the next CronWorkflow tick → the init-step resolves the new UUID and the sync completes.
- Defensive test: create two sources whose names would collide → init-step returns `ERROR: ambiguous connection name` and fails the Workflow without choosing.

## Pros and Cons of the Options

### Option A — Hard-code `connection_id` (UUID) in CronWorkflow spec

Render the per-connector CronWorkflow with the resolved Airbyte UUID at reconcile time.

- Good, because zero extra API call per sync run.
- Good, because simplest to write.
- Bad, because breaks on every recreate-with-state event.
- Bad, because requires reconcile to detect UUID drift and re-render the CronWorkflow on every change.
- Bad, because there is a race window where the next scheduled tick fails before reconcile has re-rendered.

### Option B — Resolve `connection_name → connection_id` at Workflow submit time

Per-connector CronWorkflow stores only `connection_name`. The `airbyte-sync` WorkflowTemplate gets a new init-step `resolve-connection-by-name` calling `ab_list_connections` at every Workflow submission. Lookup miss fails the Workflow.

- Good, because survives recreate transparently.
- Good, because standard k8s indirection pattern.
- Good, because aligns with `cpt-insightspec-fr-name-based-connection-resolve`.
- Neutral, because adds one extra API call per submission per connection.
- Bad, because depends on the naming uniqueness invariant (already enforced by reconcile, but now load-bearing for sync execution too).

### Option C — Custom Kubernetes CRD with controller

Introduce an `AirbyteConnection` CRD whose controller maintains the connection_id ↔ name mapping and reconciles drift.

- Good, because most "k8s-native" of the three.
- Bad, because a controller + CRD lifecycle to operate, build, deploy, and version.
- Bad, because disproportionate to the present scope — reuse of existing Argo machinery is faster.

## More Information

- The `airbyte-sync` WorkflowTemplate gains the new init-step in Phase 17 (Helm).
- The init-step image is the existing toolbox image already used elsewhere; no new build artifact.
- Validation of name uniqueness happens at Workflow submit time, not reconcile time — a future iteration may add a pre-flight check during reconcile.
- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — defines when recreate-with-state is triggered.
  - `cpt-insightspec-adr-cron-self-run-with-file-persistent-logs` (ADR-0006) — drives the per-connector CronWorkflow lifecycle.
  - `cpt-insightspec-adr-auto-trigger-sync-on-data-change` (ADR-0008) — uses the same name resolver in the one-shot Workflow path.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md) §3.13

This decision directly addresses:

- `cpt-insightspec-fr-name-based-connection-resolve` — the FR.
- `cpt-insightspec-fr-cascade-delete-cronworkflow` — relies on the named-CronWorkflow lifecycle this ADR enables.
- `cpt-insightspec-component-argo-name-resolver` — the component that implements the init-step.
- `cpt-insightspec-component-argo-cronworkflow-renderer` — the component that emits specs storing `connection_name`.
- `cpt-insightspec-seq-resolve-connection-by-name` — the sequence in DESIGN §3.6.
