---
status: superseded
date: 2026-05-07
superseded-date: 2026-05-21
superseded-by: cpt-insightspec-adr-descriptor-images-block
decision-makers: platform-engineering
---

> **SUPERSEDED 2026-05-21 by ADR-0016 (`cpt-insightspec-adr-descriptor-images-block`).** The top-level `descriptor.yaml.cdk_image:` field is removed; CDK image references now live under `descriptor.yaml.images.cdk.image` per the map-style schema. The principle of "descriptor is the single source of truth for the CDK image; reconcile never builds at runtime" is preserved and tightened in ADR-0016. This file is retained for historical context.

# ADR-0011: CDK connector images are pre-built; descriptor carries the full image reference, reconcile never builds at runtime


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Revision history](#revision-history)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Option A — Per-run docker build inside reconcile pod](#option-a--per-run-docker-build-inside-reconcile-pod)
  - [Option B — Pre-built ghcr images, reconcile derives image path](#option-b--pre-built-ghcr-images-reconcile-derives-image-path)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-insightspec-adr-cdk-prebuilt-images`

## Context and Problem Statement

CDK connectors are Python projects packaged as Docker images. An earlier prototype suggested per-run `docker build` inside the reconcile loop — that's slow, requires Docker socket access in the cron pod, and conflates build-time and runtime concerns. The deployed setup uses pre-built images published once per release to a container registry. Reconcile only needs to register/update the source_definition pointing at that image. Where does the build happen, and how does reconcile know the `dockerRepository` + `dockerImageTag` for `source_definitions/create_custom` / `source_definitions/update`?

## Decision Drivers

- **Separation of build-time and runtime** — reconcile MUST NOT mount the Docker socket or invoke `docker build` from the cron pod.
- **Speed** — reconcile loop runs every 15 min; image builds (~minutes) cannot block it.
- **Single source of truth for image identity** — one descriptor field, no env propagation, no naming convention.
- **Operator UX parity** — adding a new CDK connector should require one PR (Python project + Dockerfile + descriptor.yaml + CI tag), no manual cluster ops.
- **Registry portability** — the descriptor is the authoritative answer; image name and tag have no required structure.

## Considered Options

- **Option A** — Per-run docker build inside reconcile pod.
- **Option B** — Pre-built images, reconcile reads the full image reference from descriptor (CHOSEN).

## Decision Outcome

Chosen option: **Option B — Pre-built images, descriptor carries the full image reference**.

> **Decision (revised again, 2026-05-07)**: A `type=cdk` descriptor declares its Docker image with a single field `cdk_image` containing the **full image reference** (registry + repository + tag). The reconcile loop reads this string and splits it into Airbyte's `dockerRepository` and `dockerImageTag` using the canonical Docker reference grammar — the last `:` after the last `/` separates the tag (or `@sha256:...` digest if present). No derivation, no convention, no env var: the descriptor is the single source of truth for the image identity.
>
> Examples:
> ```yaml
> type: cdk
> cdk_image: "ghcr.io/cyberfabric/source-bitbucket-cloud-insight:2026.04.21.16.10-b36cf42"
> ```
> ```yaml
> type: cdk
> cdk_image: "registry.internal.example.com:5000/team-a/conn-007:v1.2.3"  # custom registry, custom name
> ```
>
> The image name and tag have no required structure — the descriptor is the authoritative answer. `descriptor.version` remains the Insight semantic version (audit / Argo labels), separate from the image identity.

**Justification**:

- `dockerRepository`, `dockerImageTag` derived purely by splitting `cdk_image` per Docker reference grammar (digest `@sha256:` or last `:` after last `/`); if no tag, treated as `:latest`.
- No `IMAGE_REGISTRY` env var; no `source-${name}-insight` naming convention.
- `lib/cdk-build.sh` retains its build/push/load-into-Kind subcommands for local-dev workflow; it does NOT run inside the reconcile loop.

### Revision history

- **2026-05-07 (initial)**: stated `dockerImageTag = descriptor.yaml.version`. Superseded — see Decision (revised) below.
- **2026-05-06 → 2026-05-07**: The earlier draft used a separate `image_tag` field with code deriving `dockerRepository = ${IMAGE_REGISTRY}/source-${connector}-insight`. Replaced with single `cdk_image` field carrying the full image reference. `IMAGE_REGISTRY` env removed. See FEATURE algo `cpt-insightspec-algo-reconcile-create-cdk-definition` for the split logic.

### Consequences

- **Good**, because reconcile pod stays minimal — no Docker socket, no build dependencies.
- **Good**, because there is exactly one source of truth for the image identity (the descriptor); no env propagation across local-dev / CI / Helm.
- **Good**, because the image name can be anything — digest-pinned, semantic-tagged, commit-SHA-tagged, user-friendly, or opaque — with no naming convention to enforce.
- **Good**, because `cdk_image` and `version` evolve independently; CI/CD can publish a new image without forcing a fake `version` bump, and a manifest tweak doesn't force a fake image rebuild.
- **Bad**, because authors must write the full reference string (longer than just a tag). Mitigated by the fact that they'd otherwise have to write the registry URL somewhere else anyway.
- **Neutral**, because `reconcile_definitions` for `type=cdk` + missing definition: split `cdk_image` → `source_definitions/create_custom` → register.
- **Neutral**, because tag bump in `cdk_image` (same `dockerRepository`, new `dockerImageTag`) → reconcile sees drift → `ab_set_definition_image_tag(definition_id, new_tag)`. Pod for next sync pulls new tag.

### Confirmation

- `reconcile-connectors.sh` against a clean cluster with at least one `type=cdk` descriptor publishes a definition whose `${dockerRepository}` + `${dockerImageTag}` recompose to exactly the descriptor's `cdk_image` value — `${dockerRepository}:${dockerImageTag}` for tag-pinned refs, `${dockerRepository}@${dockerImageTag}` when `dockerImageTag` is a `sha256:…` digest (digest discriminated by the `sha256:` prefix; the splitter at `python/split_docker_image_ref.py` preserves the separator information).
- Bumping `cdk_image` (tag-only or digest-only change) for a CDK connector triggers exactly one `source_definitions/update` (image-tag-only); subsequent runs report `noop`. `descriptor.yaml.version` for cdk does NOT trigger any `source_definitions/update` call (metadata-only).
- Reconcile pod runs without Docker socket mount; `kubectl get pod insight-reconcile-loop-* -o yaml | grep -i docker.sock` returns empty.

## Pros and Cons of the Options

### Option A — Per-run docker build inside reconcile pod

Reconcile loop sees a CDK descriptor → invokes `docker build` against the connector's Python project → pushes ephemerally → registers definition pointing at the freshly-built image.

- Good, because no separate CI pipeline needed for CDK image publication.
- Bad, because requires Docker socket access in the cron pod (security exposure).
- Bad, because slow — image builds add minutes to every reconcile tick on cold caches.
- Bad, because conflates build-time and runtime concerns; harder to debug when builds fail mid-reconcile.

### Option B — Pre-built ghcr images, reconcile derives image path

Images published by an out-of-band CI pipeline. Reconcile reads `cdk_image` from descriptor and splits it into `(dockerRepository, dockerImageTag)` per Docker reference grammar.

- Good (revised), because there is one source of truth for image identity. No env propagation, no naming convention. Image name can be anything (digest-pinned, semantic-tagged, commit-SHA-tagged, user-friendly, opaque — all OK).
- Good, because decoupled from `IMAGE_REGISTRY` env — no risk of the env being unset on a developer machine pulling a different image than CI's.
- Good, because reuses the existing CI/release path connector authors already use for tag-driven publication.
- Good, because version-bump is purely an Airbyte API call (`source_definitions/update`).
- Neutral, because adding a new CDK connector requires CI plumbing once per connector.
- Bad, because authors must write the full string (longer than just a tag). Mitigated by the fact that they'd otherwise have to write the registry URL somewhere else anyway.
- Bad, because if the image is not yet pushed when `cdk_image` is bumped, the next sync fails on `ImagePullBackOff`. Mitigation: PR check that the image exists in the registry before merge; reconcile WARN+skips when `cdk_image` is absent on `type=cdk`.

## More Information

- For `type=cdk`, the image-bump algorithm reads `descriptor.cdk_image` → splits → `dockerRepository` + `dockerImageTag` via `source_definitions/update`. For `type=nocode`, the version-bump algorithm reads `descriptor.version` → `declarativeManifest.description` via `connector_builder_projects/update_active_manifest`. The two algorithms operate on different descriptor fields and different Airbyte endpoints.
- `lib/cdk-build.sh` retains its `cdk_build` subcommand for local-dev workflows (operator-invoked).
- Related decisions:
  - `cpt-insightspec-adr-version-driven-reconcile` (ADR-0001) — overall reconcile flow.
  - `cpt-insightspec-adr-airbyte-workspace-as-namespace` (ADR-0009) — `custom: true` filter.
  - `cpt-insightspec-adr-nocode-via-builder-projects` (ADR-0010) — sister ADR for nocode.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **FEATURE-reconcile**: [feature-reconcile/FEATURE.md](../feature-reconcile/FEATURE.md) — flows + algos.

This decision directly addresses:

- `cpt-insightspec-fr-version-driven-reconcile` — version-bump endpoint behind the algorithm for `type=cdk`.
- `cpt-insightspec-fr-register-definitions` — registration path for CDK connectors.
- `cpt-insightspec-component-reconcile-engine` — the component that splits `cdk_image` and calls Airbyte.
- `cpt-insightspec-flow-reconcile-publish-cdk-definition` — the flow that consumes this decision.
- `cpt-insightspec-algo-reconcile-create-cdk-definition` — the algo that POSTs to `source_definitions/create_custom`.
