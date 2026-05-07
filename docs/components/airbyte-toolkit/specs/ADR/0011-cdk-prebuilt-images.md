---
status: accepted
date: 2026-05-07
decision-makers: platform-engineering
---

# ADR-0011: CDK connector images are pre-built in ghcr.io; reconcile derives dockerRepository, never builds at runtime


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
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

CDK connectors are Python projects packaged as Docker images. An earlier prototype suggested per-run `docker build` inside the reconcile loop — that's slow, requires Docker socket access in the cron pod, and conflates build-time and runtime concerns. The deployed setup uses pre-built images published once per release to GitHub Container Registry: `ghcr.io/cyberfabric/source-${connector}-insight:${version}`. Reconcile only needs to register/update the source_definition pointing at that image. Where does the build happen, and how does reconcile derive the `dockerRepository` + `dockerImageTag` for `source_definitions/create_custom` / `source_definitions/update`?

## Decision Drivers

- **Separation of build-time and runtime** — reconcile MUST NOT mount the Docker socket or invoke `docker build` from the cron pod.
- **Speed** — reconcile loop runs every 15 min; image builds (~minutes) cannot block it.
- **Single source of truth for version** — descriptor.yaml.version drives both nocode and CDK paths.
- **Operator UX parity** — adding a new CDK connector should require one PR (Python project + Dockerfile + descriptor.yaml + CI tag), no manual cluster ops.
- **Environment portability** — `IMAGE_REGISTRY` is configurable per cluster (no silent default; fail-fast if unset).

## Considered Options

- **Option A** — Per-run docker build inside reconcile pod.
- **Option B** — Pre-built ghcr images, reconcile reads `(name, version)` from descriptor and derives the full image path (CHOSEN).

## Decision Outcome

Chosen option: **Option B — Pre-built ghcr images, reconcile derives image path**.

**Justification**:

- `dockerRepository = ${IMAGE_REGISTRY}/source-${connector}-insight` (e.g. `ghcr.io/cyberfabric/source-m365-insight`).
- `dockerImageTag = ${descriptor.yaml.version}` (string; semantic version label like `2026.05.04`).
- `IMAGE_REGISTRY` is a required env (no silent default), supplied via Helm value `ingestion.reconcile.imageRegistry`.
- `lib/cdk-build.sh` retains its build/push/load-into-Kind subcommands for local-dev workflow; it does NOT run inside the reconcile loop.

### Consequences

- **Good**, because reconcile pod stays minimal — no Docker socket, no build dependencies.
- **Good**, because version-bump on a CDK connector is a 1-line descriptor change once the image tag exists in ghcr.
- **Good**, because parity with nocode lifecycle: descriptor.yaml.version is the only operator-edited field.
- **Bad**, because adding a new CDK connector requires (a) a Python project + Dockerfile in the repo, (b) a CI pipeline that builds + pushes to ghcr on tag, (c) a `descriptor.yaml` with `type: cdk` + `version` matching the published tag.
- **Neutral**, because `reconcile_definitions` for `type=cdk` + missing definition: derive image path → `source_definitions/create_custom` → register.
- **Neutral**, because version bump (`descriptor.yaml.version: X → Y`) → reconcile sees drift → `ab_set_definition_image_tag(definition_id, Y)`. Pod for next sync pulls `:Y`.

### Confirmation

- `reconcile-connectors.sh` against a clean cluster with at least one `type=cdk` descriptor publishes a definition whose `dockerRepository` matches `${IMAGE_REGISTRY}/source-${connector}-insight` and `dockerImageTag` equals `descriptor.yaml.version`.
- Bumping `descriptor.yaml.version` for a CDK connector triggers exactly one `source_definitions/update` (image-tag-only); subsequent runs report `noop`.
- Reconcile pod runs without Docker socket mount; `kubectl get pod insight-reconcile-loop-* -o yaml | grep -i docker.sock` returns empty.

## Pros and Cons of the Options

### Option A — Per-run docker build inside reconcile pod

Reconcile loop sees a CDK descriptor → invokes `docker build` against the connector's Python project → pushes ephemerally → registers definition pointing at the freshly-built image.

- Good, because no separate CI pipeline needed for CDK image publication.
- Bad, because requires Docker socket access in the cron pod (security exposure).
- Bad, because slow — image builds add minutes to every reconcile tick on cold caches.
- Bad, because conflates build-time and runtime concerns; harder to debug when builds fail mid-reconcile.

### Option B — Pre-built ghcr images, reconcile derives image path

Images published to `ghcr.io/cyberfabric/source-${connector}-insight:${version}` by an out-of-band CI pipeline on tag. Reconcile reads `(name, version)` from descriptor and constructs the full path.

- Good, because reconcile stays minimal (no Docker, no build deps).
- Good, because reuses the existing CI/release path connector authors already use for tag-driven publication.
- Good, because version-bump is purely an Airbyte API call (`source_definitions/update`).
- Neutral, because adding a new CDK connector requires CI plumbing once per connector.
- Bad, because if the image is not yet pushed when descriptor.version is bumped, the next sync fails on `ImagePullBackOff`. Mitigation: PR check that the tag exists in ghcr before merge.

## More Information

- The version-bump algorithm itself (descriptor.version → image tag) mirrors the nocode path (descriptor.version → declarativeManifest.description); only the API endpoint differs (`source_definitions/update` vs `connector_builder_projects/update_active_manifest`).
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
- `cpt-insightspec-component-reconcile-engine` — the component that derives the image path and calls Airbyte.
- `cpt-insightspec-flow-reconcile-publish-cdk-definition` — the flow that consumes this decision.
- `cpt-insightspec-algo-reconcile-create-cdk-definition` — the algo that POSTs to `source_definitions/create_custom`.
