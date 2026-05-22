---
status: superseded
date: 2026-05-13
superseded-date: 2026-05-21
superseded-by: cpt-insightspec-adr-descriptor-images-block
decision-makers: platform-engineering
---

> **SUPERSEDED 2026-05-21 by ADR-0016 (`cpt-insightspec-adr-descriptor-images-block`).** The top-level `descriptor.yaml.enrich_image:` field is removed; the enrich sidecar image reference now lives under `descriptor.yaml.images.enrich.image` per the map-style schema. The principle — "enrich image is sourced from the descriptor, never from Helm values" — is preserved in ADR-0016 (the workflow reads `images.enrich.image` at submission time via reconcile). This file is retained for historical context.

# ADR-0014: Enrich Sidecar Image in Descriptor

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Helm values (`ingestion.jiraEnrichImage`)](#option-a--helm-values-ingestionjiraenrichimage)
  - [Option B — `descriptor.yaml.enrich_image`](#option-b--descriptoryamlenrichimage)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-enrich-image-in-descriptor`

## Context and Problem Statement

Some connectors (currently jira; planned: youtrack) run an *enrich* sidecar between Airbyte sync and dbt-silver — a containerized binary that reshapes bronze rows into staging tables that dbt can consume. The enrich step has a versioned image of its own, independent of the source-definition (Builder manifest for nocode, CDK image for cdk).

Today the jira-enrich image version is declared in three places:

1. `charts/insight/values.yaml:144` — `ingestion.jiraEnrichImage`
2. `.github/workflows/build-images.yml:317-319` — CI patches the values file
3. `dev-up.sh:342,415` — local dev injects `JIRA_ENRICH_IMAGE_REF`

The connector's `descriptor.yaml` says nothing about it. As a result, the `descriptor` is *not* a single source of truth for the connector — the operator who edits `descriptor.yaml` cannot tell, by reading it, which enrich image is being deployed alongside it. A version mismatch between the manifest and the enrich binary can silently produce wrong silver output.

The CDK story (ADR-0011) already established the right pattern: `descriptor.cdk_image: ghcr.io/.../source-X:1.2.3` is the single source of truth for the cdk image. The enrich sidecar deserves the same treatment.

## Decision Drivers

- **Single source of truth**: a connector's complete identity — manifest version, image references, schedule, dbt scope — must be discoverable by reading one file.
- **PR reviewability**: a change to the enrich binary version must show up in the diff of `descriptor.yaml`, not buried in `values.yaml` or a CI patch.
- **No silent defaults**: per project rule, the Helm chart must not paper over a missing image with a default — the descriptor must declare it explicitly when the connector has an enrich step.
- **Generalize to youtrack**: the mechanism must extend to youtrack-enrich without re-architecting.

## Considered Options

- **Option A** — Status quo: keep `ingestion.jiraEnrichImage` in Helm values; CI patches it.
- **Option B** — New `descriptor.yaml.enrich_image` field, full image ref (same shape as `cdk_image`).

## Decision Outcome

Chosen option: **Option B — `descriptor.yaml.enrich_image`**.

The field is an optional, free-form image reference (registry/repo:tag-or-digest). When present, reconcile reads it and passes the value to the `ingestion-pipeline` Argo WorkflowTemplate as the `<enrich>_image` parameter at sync-trigger time. The `tt-enrich-jira-run` WorkflowTemplate no longer carries a Helm-time default; the parameter is supplied per submission from the descriptor.

- `charts/insight/values.yaml` removes `ingestion.jiraEnrichImage`.
- `charts/insight/values.schema.json` removes the corresponding entry.
- `.github/workflows/build-images.yml` patches `src/ingestion/connectors/task-tracking/jira/descriptor.yaml`'s `enrich_image` field instead of `values.yaml`.
- `dev-up.sh` stops injecting `JIRA_ENRICH_IMAGE_REF` into Helm values; either the operator edits the descriptor manually or the existing build step patches it.

### Consequences

- **Good**, because operators read one file to understand a connector's full deployment surface.
- **Good**, because the diff of an enrich-binary bump now appears next to the manifest changes that motivated it.
- **Good**, because the mechanism extends to youtrack the day its enrich script lands — add `enrich_image` to its descriptor, done.
- **Good**, because Helm-time defaults are eliminated for an inherently per-connector value, consistent with the no-silent-defaults rule.
- **Bad**, because reconcile must now read another field from each descriptor and propagate it into Argo submissions; one more wire in the plumbing.
- **Bad**, because connectors *without* an enrich step (most of them) carry an unused absent field — but YAML omission is idempotent.

### Confirmation

- After this ADR lands, `grep jiraEnrichImage charts/ src/ .github/` returns no hits.
- `descriptor.yaml.enrich_image` exists on jira, absent on every other connector.
- A sync triggered for jira reaches `tt-enrich-jira-run` with `jira_enrich_image` equal to the descriptor value.

## Pros and Cons of the Options

### Option A — Helm values (`ingestion.jiraEnrichImage`)

The version lives in `charts/insight/values.yaml` and is wired into both `ingestion-pipeline` and `tt-enrich-jira-run` Workflow templates as a Helm-time default.

- Good, because it's how things were on day one and CI knows how to update it.
- Good, because the Helm template `required` clause catches a missing value at install time.
- Bad, because the descriptor isn't authoritative — there's no signal in `descriptor.yaml` that an enrich step exists for this connector, let alone which version of it.
- Bad, because adding a second enrich-bearing connector (youtrack) duplicates the pattern in Helm rather than declaring it per-connector.

### Option B — `descriptor.yaml.enrich_image`

A `enrich_image: <full ref>` field, optional, declared next to `cdk_image` / `version`. Reconcile reads it via `parse_descriptor.py` and passes it as a submission parameter to the connector's pipeline template.

- Good, because the descriptor becomes the single, complete declaration of the connector.
- Good, because consistent with `cdk_image` precedent (ADR-0011).
- Good, because per-connector declaration generalizes naturally to youtrack and any future enrich connectors.
- Bad, because more wiring (descriptor → reconcile → sync-trigger → workflow input parameter).
- Bad, because operators must remember to bump `enrich_image` when CI rebuilds the binary — same discipline as `version`, but for a second field.

## More Information

- Related decisions:
  - `cpt-insightspec-adr-cdk-prebuilt-images` (ADR-0011) — establishes the descriptor-as-single-source-of-truth precedent for connector images.
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — descriptor as the authoritative input; this ADR extends the principle to the enrich sidecar.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE**: [FEATURE.md](../feature-reconcile/FEATURE.md)

This decision addresses:

- `cpt-insightspec-fr-descriptor-images-block` — at the time of this ADR the requirement was scoped narrowly to `descriptor.yaml.enrich_image`; ADR-0016 broadened it to the map-style `descriptor.yaml.images:` block (sole source of truth for every connector image kind), at which point the FR was renamed.
