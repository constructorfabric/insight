---
status: accepted
date: 2026-05-05
decision-makers: platform-engineering
---

# ADR-0008: Auto-Trigger Sync only on Data-Affecting Reconcile Actions


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Always trigger one sync per reconcile iteration](#option-a--always-trigger-one-sync-per-reconcile-iteration)
  - [Option B — Trigger only on data-affecting changes via `DATA_CHANGED` flag](#option-b--trigger-only-on-data-affecting-changes-via-datachanged-flag)
  - [Option C — Wait for next cron tick](#option-c--wait-for-next-cron-tick)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-auto-trigger-sync-on-data-change`

## Context and Problem Statement

The cron-driven reconcile loop (ADR-0006) applies state on every `*/15` tick, but the per-connector sync schedule is decoupled (default daily 00:00 UTC, overridable). The schedule lives in the per-connector Argo `CronWorkflow` rendered by `cpt-insightspec-algo-reconcile-render-cron-workflow` — Airbyte connections themselves are created with `scheduleType=manual` so the Temporal scheduler does not fire syncs in parallel with Argo (a parallel fire would land Bronze rows without running dbt → Silver). A real operational pattern is: an operator rotates a credential at 14:00; without an immediate sync the next data refresh waits until 00:00 UTC, ~10h later. We need a deterministic "fire one sync Workflow now" path on changes that meaningfully affect produced data, while keeping idle ticks free of spurious Workflows.

What counts as "meaningful"? The reconcile loop performs many actions:

- definition publish (version bump or breaking config change)
- source create / update (K8s Secret data changed; cfg-hash mismatch)
- connection create
- connection update — tag-only
- connection update — syncCatalog (breaking drift → recreate-with-state)
- definition update — `description`-only (cosmetic)
- garbage collection of orphans

Of these, only definition publish, source create/update, connection create, and connection-recreate-with-state actually change what the next sync will produce. Tag-only and description-only patches change nothing the sync sees.

How do we encode "data-affecting vs not" in a way that survives future diff types?

## Decision Drivers

- **Determinism**: an operator must be able to predict, from a reconcile diff, whether a sync Workflow will be submitted.
- **Queue protection**: tag/description churn must NOT flood Airbyte's sync queue.
- **Steady-state heartbeat**: per-connector CronWorkflow continues to be the long-term schedule; one-shot triggers are additive.
- **Fail-safe default**: a forgotten classification on a new diff type must default to NOT firing (no surprise burst).
- **Operator override**: a `--no-sync-trigger` flag must exist for cluster-bringup and cleanup scenarios.

## Considered Options

- **Option A** — Always trigger one sync per reconcile iteration that touches a connector.
- **Option B** — Trigger only on data-affecting changes via a per-connector `DATA_CHANGED` flag (CHOSEN).
- **Option C** — Wait for the next per-connector cron tick; never fire a one-shot Workflow.

## Decision Outcome

Chosen option: **Option B — trigger only on data-affecting changes**.

**Justification**: `lib/reconcile.sh` carries a `DATA_CHANGED` flag per connector, set/cleared by explicit classification rules. When set at the end of a connector iteration, `lib/argo.sh:argo_submit_sync_trigger` renders `templates/sync-trigger.yaml.tpl` and `kubectl create -f -` a one-shot Workflow. The per-connector CronWorkflow continues to run on its schedule; the one-shot is additional, not a replacement.

Data-affecting triggers (set `DATA_CHANGED`):

1. `descriptor.yaml.version` bump — `cpt-insightspec-algo-reconcile-diff-definition-version` reports `differ`.
2. K8s Secret data change — `cpt-insightspec-algo-reconcile-diff-source-config` reports `differ` (cfg-hash mismatch).
3. New connector or connection created in this iteration.
4. Recreate-with-state on breaking syncCatalog drift.

Non-data-affecting exclusions (do NOT set `DATA_CHANGED`):

1. Tag-only patches (`cpt-insightspec-algo-reconcile-diff-connection-tags` is the only diff).
2. `definition.description`-only patches (cosmetic version-anchor refresh).

### Consequences

- **Good**, because deterministic flush after credential rotations and schema changes.
- **Good**, because tag and description churn does not flood Airbyte's sync queue.
- **Good**, because per-connector CronWorkflow remains the steady-state heartbeat.
- **Bad**, because new diff types must be explicitly classified (data-affecting vs not). Implementations that forget to classify default to NOT firing — fail-safe — but produce surprised operators until the classification is added.
- **Bad**, because bursty bootstrap (a fresh cluster with N connectors) submits N parallel one-shot Workflows on the first tick.

### Confirmation

- DoD `cpt-insightspec-dod-reconcile-sync-triggers-only-on-data-change` (FEATURE-reconcile, Phase 7): apply tag-only patch to descriptor → reconcile → no `airbyte-sync` Workflow created. Apply version bump → reconcile → exactly one Workflow created.
- Idempotency harness (Phase 18): 100 quiet runs produce ZERO new Workflows.

## Pros and Cons of the Options

### Option A — Always trigger one sync per reconcile iteration

Every iteration that touches an Airbyte resource fires one sync Workflow per touched connector.

- Good, because simplest rule.
- Bad, because doubles Airbyte queue depth on noisy iterations (e.g., bulk tag rename).
- Bad, because wastes cluster resources on cosmetic edits.
- Bad, because defeats the cron-on-schedule design (the per-connector schedule becomes redundant).

### Option B — Trigger only on data-affecting changes via `DATA_CHANGED` flag

Reconcile classifies each connector iteration as data-affecting or not. One-shot Workflow fires IFF data-affecting.

- Good, because deterministic, explainable rule.
- Good, because no spurious syncs.
- Good, because per-connector CronWorkflow remains the long-term schedule.
- Good, because aligned with `cpt-insightspec-fr-auto-trigger-sync-on-data-change`.
- Neutral, because requires careful classification of every reconcile action.
- Bad, because new diff types must be classified explicitly when added (mitigated by fail-safe default).

### Option C — Wait for next cron tick

Never fire a one-shot Workflow; rely entirely on the per-connector schedule.

- Good, because simplest implementation.
- Bad, because 10-12h freshness lag on credential rotations.
- Bad, because operationally painful when fixing live data issues — operators end up bypassing reconcile and running Workflows by hand.

## More Information

- A future iteration may add a `--skip-sync-trigger` operator override (Phase 14 plans `--no-sync-trigger`).
- Diff classification is documented in DESIGN §3.13 and FEATURE-reconcile §3.
- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — defines the version-bump trigger that this ADR consumes.
  - `cpt-insightspec-adr-cron-self-run-with-file-persistent-logs` (ADR-0006) — provides the cron pod that fires one-shot Workflows from this ADR's policy.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md) §3.13

This decision directly addresses:

- `cpt-insightspec-fr-auto-trigger-sync-on-data-change` — the FR.
- `cpt-insightspec-component-argo-sync-trigger` — the component that submits the one-shot Workflow.
- `cpt-insightspec-algo-reconcile-render-sync-trigger` — the algorithm that renders the Workflow YAML.
- `cpt-insightspec-seq-sync-trigger-on-change` — the sequence in DESIGN §3.6.
- `cpt-insightspec-dod-reconcile-sync-triggers-only-on-data-change` — the DoD.
