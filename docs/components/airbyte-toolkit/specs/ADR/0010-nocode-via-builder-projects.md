---
status: accepted
date: 2026-05-06
decision-makers: platform-engineering
---

# ADR-0010: Insight nocode connectors managed via `connector_builder_projects` exclusively


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Continue with `create_custom` + parallel update endpoint](#option-a--continue-with-create_custom--parallel-update-endpoint)
  - [Option B — Migrate fully to `connector_builder_projects`](#option-b--migrate-fully-to-connector_builder_projects)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-nocode-via-builder-projects`

## Context and Problem Statement

Airbyte exposes two ways to register a nocode (declarative-manifest) connector. (a) `POST /api/v1/source_definitions/create_custom` with the manifest inline — creates a definition with NO linked builder project. (b) `POST /api/v1/connector_builder_projects/create` followed by `/publish` — creates both a builder project (editable in UI) and the linked source_definition. The legacy `register.sh` used path (a). Result: orphan definitions whose declarative `description` (semantic version) cannot be updated through `connector_builder_projects/update_active_manifest` because there is no builder project linked — the call returns 404. The reconcile loop's version-bump path is therefore broken for any nocode connector ever registered via the old script. We need a single canonical lifecycle for nocode connectors so that version bumps, manifest edits, and UI editability all work uniformly.

## Decision Drivers

- **Single canonical lifecycle** for nocode definitions (create → publish → update_active_manifest).
- **Compatibility with reconcile** — the version-bump algorithm (descriptor.version → declarativeManifest.description) MUST work after first registration.
- **UI editability** — operators must be able to inspect and edit the manifest in Airbyte Builder.
- **Idempotent recovery** from legacy `create_custom`-registered definitions on existing dev-vhc cluster.
- **Manifest as code** — `connectors/<area>/<name>/connector.yaml` remains the canonical source.

## Considered Options

- **Option A** — Continue with `create_custom`; build a parallel "create_custom_update" path to mutate declarative description directly.
- **Option B** — Migrate fully to `connector_builder_projects/create + publish + update_active_manifest` (CHOSEN).

## Decision Outcome

Chosen option: **Option B — `connector_builder_projects` exclusively for nocode**.

**Justification**: Airbyte's builder API is the natural lifecycle for declarative-manifest connectors — it gives a single authoritative endpoint for create, publish, and version-bump, plus UI editability for free. CDK (Docker-image) connectors are unaffected and continue to use `create_custom` via `cdk_register_definition`.

### Consequences

- **Good**, new helpers in `lib/airbyte.sh`: `ab_builder_create_with_manifest`, `ab_builder_publish`, `ab_builder_update_active_manifest`, `ab_builder_find_by_name`, `ab_builder_find_by_definition`, `ab_get_definition_description`.
- **Good**, `lib/reconcile.sh:reconcile_definitions` for `type=nocode` calls the builder API instead of `create_custom`.
- **Good**, orphan-definition recovery: when a `custom: true` definition is found without a linked builder project, reconcile emits a WARN and skips version sync for that connector — it does NOT hard-delete the definition (which would cascade-break linked sources and connections). Operators run the dedicated `tools/migrate-orphan-definition.sh <connector>` helper which exports per-connection state, recreates the definition via builder, recreates the sources + connections, and restores state. Idempotent at the API level (subsequent reconcile passes find a healthy builder + definition pair and use `update_active_manifest`).
- **Good**, `lib/cdk-build.sh:cdk_register_definition` (CDK path) keeps `create_custom` unchanged. CDK connectors don't go through builder.
- **Good**, `connectors/<area>/<name>/connector.yaml` is the canonical manifest source; loaded via `python/load_connector_manifest.py` (PyYAML → compact JSON object) before being POSTed.
- **Neutral**, one-shot migration on existing dev-vhc cluster handled by `tools/migrate-orphan-definition.sh` (state-preserving); reconcile itself only WARNs on orphans to avoid destroying linked sources/connections.
- **Bad**, builder API was introduced more recently than `create_custom`; we rely on its presence in our supported Airbyte versions. Mitigation: pin Airbyte chart version in CI.

### Confirmation

- `reconcile-connectors.sh` against a fresh workspace publishes a nocode definition + builder project; the same connector is editable in Airbyte UI.
- Bumping `descriptor.yaml.version` triggers exactly one `update_active_manifest` call; subsequent runs report `noop`.
- On dev-vhc with legacy orphan definitions, the first reconcile pass WARNs (no destructive action). Operators then run `tools/migrate-orphan-definition.sh <connector>` once per orphan to migrate state-preservingly; subsequent reconcile passes report `noop`.
- DoD `cpt-insightspec-dod-reconcile-version-bump-applied` exercises the version-bump path through builder.

## Pros and Cons of the Options

### Option A — Continue with `create_custom` + parallel update endpoint

Keep registering via `source_definitions/create_custom`; build a parallel "create_custom_update" code path to mutate declarative description directly via the underlying definition store.

- Good, because zero migration of existing definitions.
- Bad, because Airbyte does not expose a clean public endpoint to update the inline manifest of a `create_custom`-registered definition; reaching into the underlying store is unsupported and brittle.
- Bad, because no UI editability — operators cannot inspect manifests in Builder.
- Bad, because keeps two parallel registration paths alive forever.

### Option B — Migrate fully to `connector_builder_projects`

Single canonical lifecycle: `connector_builder_projects/create` → `/publish` → `/update_active_manifest`. CDK path (`create_custom`) is unchanged.

- Good, because one canonical lifecycle for nocode; one set of helpers in `lib/airbyte.sh`.
- Good, because UI editability for free.
- Good, because `update_active_manifest` is the documented Airbyte endpoint to bump the active manifest + description.
- Good, because orphan recovery is idempotent and self-healing.
- Bad, because requires a one-time deletion+republish of legacy definitions; the recovery path handles this on the next reconcile.

## More Information

- The version-bump algorithm itself (descriptor.version → declarativeManifest.description) is unchanged; only the API endpoint behind it differs.
- `connectors/<area>/<name>/connector.yaml` remains the canonical manifest source; `python/load_connector_manifest.py` parses it (PyYAML → compact JSON object) before being POSTed.
- CDK connectors continue to use `create_custom` via `cdk_register_definition`. CDK path is out of scope for this ADR.

### Airbyte 1.7+ endpoint clarification

Throughout this ADR `update_active_manifest` is used as a **semantic** label for "bump active manifest version". The actual HTTP endpoint Airbyte exposes for this operation **changed in 1.7+**:

- `POST /api/v1/connector_builder_projects/update_active_manifest` — only flips the active-version pointer to an existing version; does **not** accept manifest content.
- `POST /api/v1/declarative_source_definitions/create_manifest` — the endpoint reconcile actually calls. Adds a new manifest version (caller computes `current+1` from `connector_builder_projects/list[].activeDeclarativeManifestVersion`) and sets it active via `setAsActiveManifest: true`.

Body shape (per Airbyte server OpenAPI on 1.8.5):
```
{
  "workspaceId": "...",
  "sourceDefinitionId": "...",       // NOT builderProjectId
  "setAsActiveManifest": true,
  "declarativeManifest": {
    "description": "<descriptor.version>",
    "manifest": <full manifest JSON>,
    "spec": {                        // wrapper, NOT raw manifest.spec
      "documentationUrl": "<from manifest.spec.documentation_url>",
      "connectionSpecification": <from manifest.spec.connection_specification>,
      "advancedAuth": <from manifest.spec.advanced_auth, optional>
    },
    "version": <current+1>
  }
}
```

The first publish (when no source-def exists yet) still goes through `connector_builder_projects/create` → `/publish` (which takes the same `spec` wrapper inside `initialDeclarativeManifest`). The bash helper [ab_builder_update_active_manifest](../../../../../src/ingestion/reconcile-connectors/lib/airbyte.sh) hides this distinction — callers pass `source_definition_id` and the function picks the right endpoint and body.
- Related decisions:
  - `cpt-insightspec-adr-airbyte-workspace-as-namespace` (ADR-0009) — workspace identity that the builder API operates within.
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — version-bump cadence; this ADR changes the API behind it.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE-reconcile**: [feature-reconcile/FEATURE.md](../feature-reconcile/FEATURE.md) — flows + algos.

This decision directly addresses:

- `cpt-insightspec-fr-register-definitions` — registration path for nocode connectors.
- `cpt-insightspec-fr-version-driven-reconcile` — version-bump endpoint behind the algorithm.
- `cpt-insightspec-component-registrar` — the component owning nocode registration.
- `cpt-insightspec-component-reconcile-engine` — orphan-recovery loop.
