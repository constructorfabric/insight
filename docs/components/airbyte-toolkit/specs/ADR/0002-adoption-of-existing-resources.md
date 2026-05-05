---
status: accepted
date: 2026-05-04
decision-makers: platform-engineering
---

# ADR-0002: Adoption of Existing Airbyte Resources via Tag Annotation


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Recreate all resources](#option-a--recreate-all-resources)
  - [Option B — Rename to canonical pattern with version-encoded names](#option-b--rename-to-canonical-pattern-with-version-encoded-names)
  - [Option C — Tag-based adoption (membership + cfg-hash)](#option-c--tag-based-adoption-membership--cfg-hash)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-adoption-of-existing-resources`
## Context and Problem Statement

Legacy clusters (e.g., virtuozzo, observed 2026-05-04) already have running Airbyte sources and connections that were created by the pre-refactor `register.sh` and `connect.sh` scripts. Each connection has accumulated Airbyte sync state (per-stream cursors). The new reconcile engine introduced by ADR-0001 expects each connection to carry an `insight` membership tag plus a `cfg-hash:<sha256(secret.data)>` tag, and each definition to carry the descriptor version in its `description` field — none of which exist on legacy resources.

How do we bring legacy resources under the new declarative model **without** recreating any source or connection — i.e., without losing sync state and without forcing a full historical re-fetch?

## Decision Drivers

- **Zero sync-state loss**: connections must keep their accumulated cursors. Recreating a connection drops state silently.
- **Idempotent re-runs**: an adoption pass that fails partway must be safely re-runnable; running it twice on a fully-adopted set must be a no-op.
- **Auditability**: after adoption, an operator must be able to identify "our" resources from "not-ours" via a single deterministic query.
- **Minimal operator effort**: adoption should be a single command, not a hand-edit per resource.
- **Reversibility during rollout**: if adoption proves wrong, undo must be a tag removal, not a resource recreation.

## Considered Options

- **Option A** — Recreate all resources: delete every existing source/connection and let reconcile create fresh ones from K8s Secrets.
- **Option B** — Rename existing resources to a canonical pattern, encoding membership and version in the name string.
- **Option C** — Add Airbyte tags (`insight` membership marker + `cfg-hash:<sha256(secret.data)>`) via `PATCH /api/public/v1/connections/{id}`; patch each `definition.declarativeManifest.description` via `update_active_manifest` to mirror `descriptor.yaml.version`. No resource recreation.

## Decision Outcome

Chosen option: **Option C — tag-based adoption via `PATCH connections/{id}` and `update_active_manifest`**.

**Justification**: tags are a metadata field added to connections in Airbyte ≥ 0.50; updating tags via the public API is a metadata patch that does not invalidate the connection's `connectionId` and therefore does not touch its sync state. Updating `definition.declarativeManifest.description` via `connector_builder_projects/update_active_manifest` likewise patches metadata without invalidating sources or connections that reference the definition. Together these two operations bring a legacy resource fully under the new contract while leaving every UUID and every cursor intact.

### Consequences

- **Good**, because zero sync-state loss — connections keep their `connectionId` and Airbyte continues to read state by `connectionId`.
- **Good**, because the adoption pass is idempotent: tags are absent → set; tags already correct → no API call. Safe to re-run.
- **Good**, because membership is queryable: `GET /api/public/v1/connections?tagIds=<insight-tag-id>` returns exactly our connections.
- **Good**, because rollback is trivial: remove the tag.
- **Good**, because under `--dry-run`, the operator sees every planned patch before any state-changing call.
- **Bad**, because the contract relies on Airbyte's tags and `update_active_manifest` APIs continuing to behave non-disruptively. Mitigation: the post-adoption smoke test exercises both paths; a contract regression in Airbyte would surface immediately.
- **Bad**, because tags are workspace-scoped — if Insight ever runs in a multi-workspace Airbyte instance, the `insight` tag must be created per workspace. (Out of current scope; Airbyte is single-workspace per cluster today.)

### Confirmation

- After `reconcile-connectors.sh adopt` runs against a legacy cluster, every reconcile-managed connection has both the `insight` tag and a `cfg-hash:<sha>` tag visible in the Airbyte UI and via `GET /api/public/v1/connections/{id}`.
- A subsequent `reconcile-connectors.sh --dry-run` reports zero pending creates and zero pending recreates.
- `git diff` of the cluster state file (if any tooling captures it) shows no UUID changes — every source and connection retained its identity.

## Pros and Cons of the Options

### Option A — Recreate all resources

Delete every existing source and connection; let the new reconcile pass create them fresh from K8s Secrets and `descriptor.yaml`.

- Good, because the resulting state is uniformly compliant with the new contract — no legacy artifacts.
- Good, because there is no migration path to maintain — the new code only ever sees fresh resources.
- Bad, because every connection loses its sync state on delete, forcing a full historical re-fetch on the next run.
- Bad, because for high-volume connectors (Slack, Jira, Bitbucket history) re-fetch can take hours and may exceed source-side rate limits.
- Bad, because in-flight Airbyte jobs targeting the deleted resources fail abruptly, leaving error noise across observability.

### Option B — Rename to canonical pattern with version-encoded names

Rename existing sources and connections to a name pattern that encodes the descriptor version, e.g., `bamboohr-bamboohr-main-virtuozzo-v2026.05.04`.

- Good, because metadata stays inside the resource itself.
- Neutral, because Airbyte does not strongly index by name — querying "all our resources" becomes a substring scan.
- Bad, because rename is destructive in some Airbyte versions (the API may treat `name` change as recreate); risk of unintended state loss.
- Bad, because version is per-instance encoded into name, but version is a definition-level concept — placing it on every source is duplicate metadata.
- Bad, because operator-readable name becomes encoding-noise rather than human-friendly.

### Option C — Tag-based adoption (membership + cfg-hash)

`PATCH /api/public/v1/connections/{id}` adds tags `insight` and `cfg-hash:<sha256(secret.data)>` to each existing connection. `update_active_manifest` sets `definition.declarativeManifest.description` to the descriptor version. Sources are not modified.

- Good, because tags are a first-class Airbyte metadata field — explicit, queryable, mutable in-place.
- Good, because membership and config drift are independently expressible (one tag for "ours", another for "this credential snapshot").
- Good, because both API operations are documented as non-disruptive on the resource UUID.
- Neutral, because adoption requires a one-time run before the new reconcile mode is safe to enable.
- Bad, because behavior depends on the tags API; any future Airbyte change to tag semantics would require migration.

## More Information

- Sequence: `cpt-insightspec-seq-adopt-one-shot` in `DESIGN.md` §3.6 details the API call order.
- Tag verification (2026-05-04, virtuozzo): `POST /api/public/v1/tags` to create the `insight` tag, `PATCH /api/public/v1/connections/{id}` to attach, `GET /api/public/v1/connections?tagIds=<id>` to query — all three round-tripped successfully.
- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — provides the version anchor that adoption sets.
  - `cpt-insightspec-adr-credential-rotation-no-env` (ADR-0003) — explains why `cfg-hash` is computed from K8s Secret, not from Airbyte's masked credential view.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses:

- `cpt-insightspec-fr-adopt-legacy-resources` — the FR that defines the required behavior.
- `cpt-insightspec-fr-state-preserved-on-breaking-change` — adopting in-place is the precondition for state preservation in subsequent reconciles.
- `cpt-insightspec-component-adopt-pass` — the component that implements this ADR.
- `cpt-insightspec-component-reconcile-engine` — runs after adoption on legacy clusters.
