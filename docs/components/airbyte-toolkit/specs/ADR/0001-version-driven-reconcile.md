---
status: accepted
date: 2026-05-04
decision-makers: platform-engineering
---

# ADR-0001: Descriptor-Version-Driven Reconcile


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Local `state.yaml` file in the repo](#option-a--local-stateyaml-file-in-the-repo)
  - [Option B — Cluster-side `ConfigMap airbyte-state`](#option-b--cluster-side-configmap-airbyte-state)
  - [Option C — `descriptor.yaml.version` mirrored to Airbyte](#option-c--descriptoryamlversion-mirrored-to-airbyte)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-version-driven-reconcile`
## Context and Problem Statement

The Airbyte Toolkit reconciles a set of declared connectors (each described by `connectors/<name>/descriptor.yaml` + a K8s Secret) into Airbyte resources (definitions, sources, connections). Across many run cycles the toolkit needs a deterministic answer to "should I republish this definition?" — i.e., does what's currently in Airbyte already match what the operator intends? Without a single durable signal, the engine either republishes blindly (creating duplicate definitions and re-discovering catalogs each run) or relies on a parallel local state file that drifts from cluster reality and silently becomes wrong.

How do we represent the *intended* connector version durably and unambiguously, so reconcile can no-op when nothing has changed and act decisively when it has?

## Decision Drivers

- **Idempotency**: running reconcile twice with no operator intent change MUST be a no-op at the API call level (no republish, no recreate).
- **No recreate on version bump**: when the operator bumps the version, sources and connections must NOT be deleted and recreated by default — sync state (Airbyte cursors per stream) is precious.
- **Human-editable**: the version anchor must live in version-controlled source code so PR review and `git blame` apply.
- **Low operational overhead**: no extra storage system to provision, monitor, back up, or recover.
- **Drift-resistant**: the anchor must not depend on a parallel local state that can desynchronize from cluster reality (this is the failure mode of `state.yaml` and `airbyte-state` ConfigMap on the virtuozzo cluster as of 2026-05-04).

## Considered Options

- **Option A** — Local `state.yaml` file in the repo, with `applied_version` per connector
- **Option B** — Cluster-side `ConfigMap airbyte-state` mirroring `state.yaml`
- **Option C** — `descriptor.yaml.version` field mirrored into Airbyte (`definition.declarativeManifest.description` for nocode connectors, `dockerImageTag` for CDK connectors)

## Decision Outcome

Chosen option: **Option C — `descriptor.yaml.version` mirrored to Airbyte**.

**Justification**: the `descriptor.yaml` file is already the canonical declarative input for each connector; adding a `version` field there gives the operator one place to bump. Mirroring the value onto the Airbyte side via fields Airbyte already exposes (`description` for declarative manifests, `dockerImageTag` for Docker images) means **Airbyte itself becomes the authoritative store** for "what version is currently published". The toolkit reads back what Airbyte holds and compares to the file on disk; no parallel store is required, no ConfigMap to keep in sync. The operator continues to author in git, and reconcile becomes a function of two values rather than three.

### Consequences

- **Good**, because there is exactly one source of truth (the file on disk) and one authoritative actual state (Airbyte) — diff becomes a string comparison instead of a search across stores.
- **Good**, because removing `state.yaml` and the `airbyte-state` ConfigMap eliminates a class of drift bugs already observed in production.
- **Good**, because version is editable via PR, with diff history and review.
- **Good**, because `sources/update` and `PATCH /api/public/v1/connections/{id}` for tags do not invalidate Airbyte sync state — versioned reconcile no longer threatens cursors.
- **Bad**, because operators must remember to bump `descriptor.yaml.version` when they edit `connector.yaml` or `descriptor.yaml`. Forgetting to bump means reconcile no-ops despite real intent change. (Mitigation in §Confirmation: a pre-commit/CI check.)
- **Bad**, because Airbyte's `definition.declarativeManifest.description` was not designed as a metadata field for downstream tooling — relying on it is a soft contract that could change in a future Airbyte release. (Mitigation: a smoke test in CI verifies the field still round-trips.)

### Confirmation

- Code review of every PR that touches a `descriptor.yaml` confirms the `version` field is bumped when `connector.yaml` content changes. A pre-commit hook is recommended (out of ADR scope).
- `reconcile-connectors.sh --dry-run` against a freshly-applied version bump reports `republish: <connector>` exactly once on the next run; subsequent runs report `no-op`.
- A scripted CI check enumerates Airbyte definitions and asserts that `description` (nocode) or `dockerImageTag` (CDK) equals the file value for each known connector.

## Pros and Cons of the Options

### Option A — Local `state.yaml` file in the repo

A YAML file checked into the repo, containing `applied_version` per connector along with applied resource UUIDs.

- Good, because human-editable and reviewable in git.
- Good, because zero new infrastructure.
- Neutral, because requires a strict contract between writer (CI/CD) and reader (operators).
- Bad, because the writer must commit and push for the file to reflect reality; manual recovery edits or out-of-band Airbyte changes leave the file inconsistent.
- Bad, because in observed production (virtuozzo cluster) the file already drifted heavily from Airbyte — UUIDs in the file no longer existed in the cluster.

### Option B — Cluster-side `ConfigMap airbyte-state`

A ConfigMap in the `data` namespace mirroring `state.yaml`, hydrated by the toolkit on read and updated on write.

- Good, because closer to the actual cluster state than a repo file.
- Good, because survives operator workstation changes.
- Bad, because it adds a third store alongside the repo file and Airbyte, creating two write-write races to manage.
- Bad, because it requires a separate persistence and migration story (backup, restore, re-creation when namespace is rebuilt).
- Bad, because it does not solve the underlying issue: applied state lives in Airbyte already; mirroring it elsewhere just creates more places for it to diverge.

### Option C — `descriptor.yaml.version` mirrored to Airbyte

A semver-like string in `descriptor.yaml` (baseline `2026.05.04`); on publish, the toolkit writes the value to `definition.declarativeManifest.description` for nocode connectors or includes it in `dockerImageTag` for CDK connectors. Reconcile reads the value from Airbyte and compares to the file.

- Good, because Airbyte is already the storage of record for definitions; reusing it means one less store to manage.
- Good, because `description` and `dockerImageTag` are already mutable per the Airbyte API; updates do not invalidate dependent resources.
- Good, because the operator's mental model collapses to: "bump version → push → reconcile sees new version → republish".
- Neutral, because it relies on a soft contract with Airbyte that the description field round-trips. The same is true for any tag/label scheme on a third-party system.
- Bad, because operator discipline is required for the version bump to track real changes — but this is the only mechanism that survives across stores anyway.

## More Information

- Sequence: see `cpt-insightspec-seq-reconcile-default` in `DESIGN.md` §3.6 for end-to-end reconcile flow.
- Related decisions:
  - `cpt-insightspec-adr-adoption-of-existing-resources` (ADR-0002) — how to bring legacy clusters under this scheme without recreate.
  - `cpt-insightspec-adr-credential-rotation-no-env` (ADR-0003) — orthogonal: credentials live in K8s Secret, not in version.
  - `cpt-insightspec-adr-cluster-config-via-configmap` (ADR-0004) — cluster-level `tenant_id` configuration.
- Background — original investigation against virtuozzo cluster (2026-05-04): `state.yaml` `definitions.{connector}.id` did not match any source's `sourceDefinitionId` for 8 of 9 connectors; clear evidence the parallel-store approach had failed.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)

This decision directly addresses:

- `cpt-insightspec-fr-version-driven-reconcile` — the FR that defines the contract.
- `cpt-insightspec-fr-cli-surface` — `reconcile-connectors.sh` is built on this anchor.
- `cpt-insightspec-component-reconcile-engine` — the component that consumes the anchor.
- `cpt-insightspec-component-secret-discovery` — feeds the desired side of the comparison.
