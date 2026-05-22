---
status: accepted
date: 2026-05-21
decision-makers: platform-engineering
supersedes:
  - cpt-insightspec-adr-cdk-prebuilt-images
  - cpt-insightspec-adr-enrich-image-in-descriptor
---

# ADR-0016: Descriptor `images:` Block as Single Source of Truth

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Schema](#schema)
  - [Consumers](#consumers)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Per-kind top-level fields](#option-a--per-kind-top-level-fields)
  - [Option B — Map-style `images:` block as sole source of truth](#option-b--map-style-images-block-as-sole-source-of-truth)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-descriptor-images-block`

## Context and Problem Statement

ADR-0011 introduced `descriptor.yaml.cdk_image` as the single source of truth for CDK source images. ADR-0014 extended the principle to enrich sidecars via `descriptor.yaml.enrich_image`. Both put a top-level field per image kind on the connector's descriptor.

This pattern fails to scale. A connector may ship more than two images — a CDK source, an enrich sidecar, a one-shot bootstrap container, a future migrator. Each new image kind currently demands a new top-level field, a new ADR justifying it, new Helm wiring, and new per-job hardcodes in `.github/workflows/build-images.yml`. Even with only the existing cdk/enrich kinds, CI carries a separate hardcoded job per connector that repeats `context`/`file`/`repo` data the descriptor already states implicitly via its directory layout.

Worse, the previous design split image identity across **three** locations:

1. `descriptor.yaml.<kind>_image` — the runtime ref (read by reconcile, Helm).
2. `descriptor.yaml.images[...]` — an additive sidecar block introduced as a workaround (see prior revision of this ADR).
3. `.github/workflows/build-images.yml` — hardcoded `context`/`file`/`repo` per per-image job.

Multiple sources mean drift. The descriptor must become the **only** place where a connector's image surface is declared. Every consumer — reconcile, Helm, the enrich-workflow runner, CI builds, CI bumps, `/connector create` skill — reads from the same block.

## Decision Drivers

- **One file, all answers.** A connector author reads `descriptor.yaml` to know which images ship, where their Dockerfiles live, and which version is currently deployed. CI does the same to know what to build and where to push.
- **Free-form keys.** New image kinds (bootstrap, migrator, future enrich variants) need no spec churn — add a new key under `images:`, done.
- **No hardcoded duplication in CI.** A change to `name`/`dockerfile`/`context` in the descriptor takes effect on the next CI run without touching the workflow YAML. CI uses dynamic discovery driven by the descriptor.
- **Back-compat with reconcile and Helm at the field level, not the schema level.** Reconcile still emits a 7-field TSV with `cdk_image` and `enrich_image` columns — but those columns are now sourced from `images.cdk.image` and `images.enrich.image`. Downstream code reading the TSV is unchanged. Top-level `cdk_image:` and `enrich_image:` are removed from every descriptor.

## Considered Options

- **Option A** — Status quo: per-kind top-level fields (`cdk_image`, `enrich_image`) plus an additive sidecar `images:` list. CI hardcodes build identity.
- **Option B** — Map-style `images:` block as the **sole** declaration. Top-level fields removed. CI discovers what to build by scanning descriptors.

## Decision Outcome

Chosen option: **Option B — map-style `images:` block as sole source of truth**.

### Schema

```yaml
images:
  <key>:                                        # free-form key (cdk, enrich, bootstrap, …)
    name: <ghcr-short-name>                     # GHCR image name without registry prefix or tag
    dockerfile: ./<path-under-connector-dir>    # leading "./" mandatory
    context:    ./<path-under-connector-dir>    # leading "./" mandatory (use "." for connector root)
    image:      "<full registry/repo:tag>"      # may be empty string ("") for not-yet-published
```

**Required fields per entry**: `name`, `dockerfile`, `context`, `image`. `image` may be empty string `""` for connectors whose first CI build has not happened yet (e.g. github-copilot at adoption time).

**Reserved keys with runtime semantics**:

- `cdk` — read by reconcile to determine the CDK source image when registering an Airbyte source definition (per the pattern previously held by `cdk_image`).
- `enrich` — read by the connector's enrich workflow at submission time (per the pattern previously held by `enrich_image`).

Other keys (`bootstrap`, `migrator`, …) are allowed but have no runtime consumer until one is wired. CI builds all keys regardless.

**No top-level `cdk_image:` or `enrich_image:` fields exist.** Any descriptor carrying them is non-conforming.

### Consumers

| Consumer | Reads | When |
|---|---|---|
| `disc_load_descriptors` (reconcile) | `images.cdk.image`, `images.enrich.image` | Each reconcile run — emitted as columns 5 and 6 of the 7-field TSV (`name`, `dir`, `version`, `type`, `cdk_image`, `enrich_image`, `dbt_select`). Downstream consumers of the TSV are unchanged. |
| Reconcile source-definition registration | `images.cdk.image` (via TSV) | When creating/updating Airbyte CDK source definitions. Empty string → fail loud. |
| Enrich workflow runner (e.g. `tt-enrich-jira-run`) | `images.enrich.image` (via reconcile passing it as a Workflow parameter) | Each workflow submission. The image ref is **re-read from the descriptor on every submission** — no Helm-time bake. |
| `.github/workflows/build-images.yml` | All `images:` entries via dynamic discovery on every CI run | Discovery step scans `src/ingestion/connectors/**/descriptor.yaml`, builds a matrix `[(connector_dir, key, name, dockerfile, context)]`, filters by which entries' `context` matched a changed path, fan-out builds each. |
| CI bump step | Patches `images.<key>.image` in descriptor.yaml AND bumps `descriptor.version` by one minor increment (X.Y.Z → X.(Y+1).0) | After a successful image push. Both edits land in the same commit (no `[skip ci]`) that triggers the next workflow run, which rebuilds toolbox with the patched descriptor and publishes the chart. The version bump makes reconcile classify the diff as `bump_kind: minor` per ADR-0015 — catalog re-discovery without `dbt --full-refresh`. Dedupe by connector: a single descriptor with multiple image entries gets exactly ONE minor bump per CI run, not one per entry. |
| `/connector create` skill | Required output | Skill emits an `images:` block with one entry per Dockerfile the connector ships. Validated by `cpt validate` rule `connector-images-triad`. |
| Helm chart | Does NOT read `images:` directly. Reads `ingestion.toolboxImage` for toolbox only. Connector image refs travel inside the toolbox image (descriptor baked at build time). | At deploy. |

### Consequences

- **Good**, the descriptor is the only place to express a connector's complete image surface. No duplication, no drift.
- **Good**, adding an image kind to a connector is a descriptor edit. No new ADR, no new top-level field, no Helm wiring per kind.
- **Good**, CI is one matrix job with shared build logic. Adding a new connector with images is zero-CI-edits if the connector follows the directory and descriptor convention.
- **Good**, the enrich workflow always uses the latest declared image because reconcile re-reads it on every submission.
- **Bad**, ADR-0011 and ADR-0014 are superseded, and a migration is needed to delete the top-level `cdk_image:` / `enrich_image:` fields from every descriptor. One-shot migration; no runtime cost after.
- **Bad**, CI gains a YAML discovery step on every run (~100 ms). Negligible against image-build time.
- **Bad**, the runtime semantics of the two reserved keys (`cdk`, `enrich`) are not enforced by the schema itself; a future ADR or cpt validate rule must check that any key reconcile/enrich expects is present and non-empty.

### Confirmation

- `grep -RIn "^cdk_image\|^enrich_image" src/ingestion/connectors/` returns 0 hits (except for commented placeholders inside the `images:` block's leading explanatory comment, which is informational).
- Every connector directory containing a Dockerfile has a non-empty `images:` block in its `descriptor.yaml`. Verified by `find src/ingestion/connectors -name Dockerfile | xargs -I{} dirname {} | xargs -I{} yq '.images | length' {}/descriptor.yaml` returning ≥ 1 per directory.
- Every descriptor that declares an `images:` block has `version` set to strict semver `MAJOR.MINOR.PATCH` (no leading zeros, no pre-release, no build metadata). The CI bump step (`.github/workflows/scripts/bump-descriptor-version.py`) fails loud on non-semver values; descriptors found in violation MUST be fixed manually before CI can advance them.
- Reconcile's 7-field TSV's columns 5 and 6 match `images.cdk.image` and `images.enrich.image` when read directly from each descriptor.
- A trivial Dockerfile change pushes through two CI runs (image build + descriptor patch + minor version bump → toolbox rebuild + chart publish) ending with a new umbrella chart on GHCR; the affected descriptor's `version` field advances by one minor increment.

## Pros and Cons of the Options

### Option A — Per-kind top-level fields

- Good, because zero migration cost — the existing pattern works.
- Bad, because every new image kind needs a new top-level field and a new ADR.
- Bad, because CI hardcodes build identity per connector — N copies of essentially the same job body.
- Bad, because the descriptor lies about being a single source of truth: build identity actually lives in the workflow YAML.

### Option B — Map-style `images:` block as sole source of truth

- Good, because the descriptor is literally the source of truth — change it, CI follows.
- Good, because adding an image is a descriptor edit and nothing else.
- Good, because `/connector create` enforces a uniform schema.
- Good, because the same block drives build (CI), bump (CI), and runtime read (reconcile, enrich).
- Bad, because a one-shot migration is required to delete the legacy top-level fields and update reconcile's reader.
- Bad, because two keys (`cdk`, `enrich`) have runtime semantics not enforced by the schema. Mitigated by `cpt validate` rule `connector-images-triad`.

## More Information

- **Supersedes**: `cpt-insightspec-adr-cdk-prebuilt-images` (ADR-0011) — top-level `cdk_image:` field replaced by `images.cdk.image`. ADR-0011's status MUST be set to SUPERSEDED with `superseded-by: cpt-insightspec-adr-descriptor-images-block`.
- **Supersedes**: `cpt-insightspec-adr-enrich-image-in-descriptor` (ADR-0014) — top-level `enrich_image:` field replaced by `images.enrich.image`. ADR-0014's status MUST be set to SUPERSEDED with the same `superseded-by` reference.
- **Related**: `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — descriptor as the authoritative input. This ADR preserves and tightens that principle.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE**: [FEATURE.md](../feature-reconcile/FEATURE.md)

This decision addresses:

- `cpt-insightspec-fr-descriptor-images-block` — connector image surface declared entirely in `descriptor.yaml.images:` as a map of free-form keys to `{name, dockerfile, context, image}` entries; CI discovers and builds all entries; reconcile reads `cdk`/`enrich` from the block; no top-level `cdk_image:` / `enrich_image:` fields exist.
