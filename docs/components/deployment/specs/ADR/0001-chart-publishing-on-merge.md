---
status: accepted
date: 2026-05-07
---

# ADR-0001: Publish the Umbrella Helm Chart per Merge to `main`

**ID**: `cpt-insightspec-adr-dep-chart-publishing-on-merge`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Per-merge umbrella publish to OCI (chosen)](#per-merge-umbrella-publish-to-oci-chosen)
  - [Sibling checkout of the public repo](#sibling-checkout-of-the-public-repo)
  - [Per-service OCI chart publishing](#per-service-oci-chart-publishing)
  - [Curl scripts from GitHub at deploy time](#curl-scripts-from-github-at-deploy-time)
  - [Default every `image.tag` to the umbrella's `appVersion`](#default-every-imagetag-to-the-umbrellas-appversion)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

The umbrella Helm chart and the service container images live on independent CI timelines. Images rebuild whenever a service's source changes; chart templates change less frequently and on a different schedule. Operators consuming the chart must somehow combine "the chart shape I want" with "the image tags I want" and these were drifting:

- Engineers overrode `image.tag` per service in environment values, then a chart-template change shipped that needed a different shape than the override implied.
- A consumer pulling the chart at HEAD got chart shape from one moment and image tags from another, with no audit trail tying them together.
- Promotion across environments meant copying many image tags between values files and crossing fingers that the chart shape hadn't moved.

The deployment subsystem needs a single releasable artifact that bundles both chart shape and the image tags it expects, so that pinning one version in a gitops repo pins both. The question: how should that artifact be produced, named, and published?

## Decision Drivers

* No drift between chart shape and image tags for any deployed version
* Per-service tag granularity preserved -- rebuilding one service should not advance every other service's tag
* One pin per gitops environment -- promotion is one file change, not many
* Self-contained gitops repo -- no requirement for a sibling checkout of the public repo on the operator's workstation
* Atomic publish -- the chart artifact and its image references must be produced from one CI run on one commit
* Tooling-friendly -- standard `helm` commands, no `oras`-only paths

## Considered Options

* Per-merge umbrella publish to OCI (`oci://ghcr.io/constructorfabric/charts/insight:<semver>`)
* Sibling checkout of `constructorfabric/insight` from the gitops repo's Makefile
* Per-service OCI chart publishing (`oci://ghcr.io/constructorfabric/charts/insight-<service>:<tag>`)
* Curl scripts and chart files from GitHub at deploy time
* Default every `image.tag` to the umbrella's `.Chart.AppVersion`

## Decision Outcome

Chosen option: **per-merge umbrella publish to OCI**.

GitHub Actions, on every merge to `main` of `constructorfabric/insight`, runs one workflow that builds the changed images, bumps the affected subcharts' `appVersion` to the build tag, patch-bumps the umbrella's `version` and sets its `appVersion` to the build tag, runs `helm dependency update`, packages the umbrella, and pushes it to `oci://ghcr.io/constructorfabric/charts/insight:<umbrella-version>`. The version bumps are auto-committed back to `main` so the repo state matches what was published.

Each subchart's values default `image.tag = ""` and the templates resolve via `default .Chart.AppVersion`. **Inside a subchart, `.Chart.AppVersion` resolves to that subchart's `appVersion` -- not the umbrella's.** Per-service granularity is structural, not by convention.

The gitops repo pins one umbrella semver per environment in a one-line file (`.insight-version`). Promotion is a bump of that file.

### Consequences

* Good, because chart shape and image tags can never drift -- both come from the same CI run on the same commit.
* Good, because per-service tag granularity is preserved -- rebuilding only `api-gateway` advances only that subchart's `appVersion`, others stay on their prior tags.
* Good, because the gitops repo is fully self-contained -- one OCI pull at deploy time, no sibling checkout, no dual-repo synchronisation on operator workstations.
* Good, because promotion across environments is one file change (`.insight-version`).
* Good, because rollback is `helm rollback` (last revision) or a one-line file revert (any prior published version).
* Good, because chart consumers outside Cyberfabric get one stable artifact reference -- `oci://ghcr.io/constructorfabric/charts/insight:<version>`.
* Bad, because every commit publishes a new umbrella tag, growing the registry. Mitigated by GHCR retention policy on chart packages.
* Bad, because every commit moves `dev`'s `.insight-version` once the poller picks it up, producing continuous deploy churn for `dev`. Mitigated by `auto_envs: [dev]` -- only `dev` is auto-bumped; higher environments require an engineer-authored MR.
* Bad, because the auto-commit-back-to-main step risks a CI-recursion loop. Mitigated by `paths-ignore` on the workflow trigger or `[skip ci]` in the bump commit message; concurrency guard as backstop.
* Neutral, because the GHCR package name includes a `charts/` segment (`oci://ghcr.io/constructorfabric/charts/insight`) -- standard Helm-OCI behaviour, not configurable without `oras`.

### Confirmation

* `helm pull oci://ghcr.io/constructorfabric/charts/insight --version <V>` succeeds from any workstation -- chart is genuinely published.
* `helm template` of the pulled chart shows `image.tag` for each service equals that subchart's `appVersion`, not the umbrella's -- per-service granularity preserved.
* The auto-commit-back lands on `main` with the bumped `Chart.yaml` files; CI does not recurse into a second publish.
* On each PR-merge, exactly one new umbrella tag is published; subchart `appVersion` fields advance only for services whose source changed.

## Pros and Cons of the Options

### Per-merge umbrella publish to OCI (chosen)

GitHub Actions publishes one umbrella chart artifact per merge to `main`. Subchart `appVersion` equals each service's image tag; subchart values default `image.tag` to `.Chart.AppVersion`. Umbrella `version` is semver, patch-bumped per publish, minor on shape change. Gitops pins one umbrella semver per environment.

* Good, because eliminates chart-vs-image drift structurally.
* Good, because preserves per-service tag granularity through subchart `appVersion`.
* Good, because one pin per gitops environment.
* Good, because chart consumers outside Cyberfabric get a stable reference.
* Bad, because registry growth (mitigated by retention).
* Bad, because dev churn (mitigated by `auto_envs`).

### Sibling checkout of the public repo

The gitops repo's Makefile resolves `$CHART` to `../insight/charts/insight` -- a sibling checkout of `constructorfabric/insight`.

* Good, because no CI publishing infrastructure required.
* Good, because chart changes are visible directly in the operator's checkout.
* Bad, because every operator workstation must clone two repos and keep them in sync.
* Bad, because no atomic snapshot -- the chart at `HEAD` of the sibling checkout may not match any released image set.
* Bad, because there is no single artifact for chart consumers outside Cyberfabric to reference.
* Bad, because rollback to a prior chart shape requires `git checkout` of the public repo, which an ops engineer should not have to do for a deploy-time decision.

### Per-service OCI chart publishing

Each subchart is published independently to its own OCI repo: `oci://ghcr.io/constructorfabric/charts/insight-api-gateway:<tag>`, `…/insight-analytics:<tag>`, etc. The umbrella references them as remote dependencies.

* Good, because finest possible granularity.
* Good, because subcharts can be consumed outside the umbrella.
* Bad, because every merge that touches one service requires bump-push-bump-push: rebuild image, publish subchart, regenerate umbrella `Chart.lock`, publish umbrella. Multi-step CI per merge.
* Bad, because the umbrella's `Chart.lock` becomes a moving target across many subchart publishes -- harder to reason about which subchart versions are bundled.
* Bad, because there is no current consumer of subcharts outside the umbrella -- the granularity is unused.
* Reserved as an option for if/when subcharts genuinely need to be consumed standalone; not the right step now.

### Curl scripts from GitHub at deploy time

The gitops Makefile fetches `install-airbyte.sh`, `install-argo.sh`, and chart files from `raw.githubusercontent.com` at deploy time, then runs them locally.

* Good, because the public repo remains the single source of truth for everything.
* Bad, because the gitops repo no longer self-contained -- a network outage to `github.com` blocks deploys.
* Bad, because no atomic snapshot -- the curl'd files at deploy time may not match what was tested.
* Bad, because curl-piped-to-bash widens the trust surface for compromised intermediates.
* Bad, because the operator cannot diff what they are about to apply against what was applied last time without a side-channel snapshot.

### Default every `image.tag` to the umbrella's `appVersion`

In every subchart's values, `image.tag` defaults to the *umbrella's* `.Chart.AppVersion`, so every service's image tag is the same string per umbrella publish.

* Good, because operationally simple -- one tag, one release.
* Bad, because every merge would require rebuilding and retagging every service image, even for services whose source did not change. Either CI rebuilds every service (wasteful) or service images carry tags that do not match their actual contents (misleading).
* Bad, because per-service rollback to a prior image tag becomes impossible without overriding `image.tag` in env values, defeating the simplification.
* Bad, because emergency hotfix of one service requires bumping the umbrella's `appVersion`, which advances every other service's effective tag too.

## More Information

The companion document is the deployment SPEC at [`gitops/README.md`](../../gitops/README.md), which describes the contract in operational detail (poller behaviour, Makefile shape, repository layout). Sections [§2.3](../../gitops/README.md#23-chart-and-manifest-versioning) and [§2.4](../../gitops/README.md#24-chart-publishing) of that SPEC are the day-to-day reference; this ADR records why those rules exist.

The implementation lives in:

- `constructorfabric/insight/.github/workflows/build-images.yml` -- the per-merge publishing workflow (Phase 2 of the deploy consolidation).
- `infra/insight-gitops/.insight-version` -- the one-line semver pin.
- `infra/insight-gitops/Makefile` -- pulls the chart from OCI, pinned to `.insight-version`.
- `infra/insight-gitops/scripts/poller.sh` -- bumps `.insight-version` based on the highest semver tag at `oci://ghcr.io/constructorfabric/charts/insight`.

## Traceability

- **PRD**: [PRD.md](../PRD.md)
- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **SPEC**: [gitops/README.md](../../gitops/README.md)

This decision directly addresses the following requirements or design elements:

* `cpt-insightspec-fr-dep-umbrella-chart` -- the umbrella chart is the single deploy artifact and is now published per merge.
* `cpt-insightspec-fr-dep-chart-publishing` -- per-merge publishing of the umbrella chart from `constructorfabric/insight` CI is the mechanism this decision establishes.
* `cpt-insightspec-fr-dep-oci-distribution` -- the OCI registry as the single addressable artifact path is the distribution surface this decision selects.
