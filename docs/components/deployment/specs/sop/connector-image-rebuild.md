# SOP — Connector Image Rebuild

**Audience**: on-call platform engineers; chart-release watchers.
**Last verified**: 2026-05-21.

## Overview

When a developer pushes a change inside a connector directory that ships a Dockerfile (CDK source, enrich sidecar, future bootstrap or migrator containers), CI runs a deterministic **two-commit publication flow** that ends with a new umbrella chart published to `oci://ghcr.io/constructorfabric/charts/insight`. The gitops poller picks up the new chart version on its scheduled tick.

The flow uses **dynamic discovery** — `.github/workflows/build-images.yml` scans every `descriptor.yaml.images:` block (per ADR-0016) on every run, builds the matrix of `(connector, image-key, name, dockerfile, context)`, filters by which entries' `context` matched a changed path, and fans out a build per entry. There is no per-connector job in the workflow YAML. Adding a new connector with images is a descriptor edit and (if the directory layout differs) a paths-trigger entry — never a per-image job copypaste.

## Flow

```
Developer push (commit X — Dockerfile or sibling code in connector dir)
  │
  ▼  Run 1
build-images.yml
  ├── changes (paths-filter sees connector dir + src/ingestion/**)
  ├── discover (scan all descriptor.yaml files; build matrix; filter by changed paths)
  ├── build (matrix fan-out) — one build per matching images.<key> entry,
  │   reading repo/dockerfile/context from descriptor.yaml at job time;
  │   push :BUILD_TAG to GHCR for each
  ├── toolbox job — rebuilds (still with STALE descriptor inside; this image is dangling)
  └── bump-descriptors
       └── commit A — for each affected descriptor:
                       (1) patches images.<key>.image = ghcr.io/.../<name>:BUILD_TAG
                       (2) bumps descriptor.version by one minor (X.Y.Z → X.(Y+1).0)
                       (NO [skip ci])
       publish-chart — SKIPPED (bump-descriptors.outputs.committed=true gate)
  │
  ▼  Run 2 (triggered by commit A's push)
build-images.yml
  ├── changes (paths-filter: per-connector filters MISS due to descriptor.yaml
  │            exclusion; toolbox filter MATCHES on src/ingestion/**)
  ├── discover + build — fan-out skipped (no context match for any connector)
  ├── toolbox job — rebuilds FRESH against the patched descriptors
  └── publish-chart
       ├── values.yaml: ingestion.toolboxImage = ghcr.io/.../insight-toolbox:NEW_TAG
       ├── Chart.yaml: umbrella version patch-bumped
       ├── helm package + helm push → oci://ghcr.io/constructorfabric/charts/insight:NEW
       └── commit B — chore(release): umbrella X.Y.Z+1 (build TAG) [skip ci]
  │
  ▼
insight-gitops repo poller (scheduled, GitLab CI)
  └── .insight-version updated → deploy-dev runs make deploy ENV=dev
      → dev cluster receives the new chart with patched descriptors
      inside the new toolbox image
```

## Expected timing

| Step | Typical | Worst case |
|---|---|---|
| Image build (Run 1, one matrix entry) | 3–6 min | 10 min |
| Multiple matrix entries (parallel) | same as slowest | + 1 min for fan-out |
| bump-descriptors commit + push | < 30 s | 60 s |
| toolbox rebuild (Run 2) | 4–6 min | 12 min |
| publish-chart (helm package + push) | 1–2 min | 4 min |
| gitops poller tick (dev) | ≤ 60 min | ≤ 60 min |
| `make deploy ENV=dev` on gitops side | 2–5 min | 10 min |

End-to-end from developer push to dev cluster running new image: **typically 15–25 minutes**, dominated by the gitops poller's hourly cadence.

## Reference points (job names)

- `changes` — paths-filter + build_tag computation.
- `discover` — scans descriptors, emits matrix.
- `build` — matrix job; one run per matching `images.<key>` entry.
- `toolbox` — rebuilds on any `src/ingestion/**` change.
- `bump-descriptors` — patches `images.<key>.image` AND bumps `descriptor.version` (one minor per affected connector) in affected descriptors; commits without `[skip ci]`; produces commit A. Calls `.github/workflows/scripts/bump-descriptor-version.py`, which fails loud on non-semver `version` values.
- `publish-chart` — bumps umbrella + pushes chart; produces commit B; skipped in Run 1 if bump-descriptors committed.

## Image identity (where does the build know what to build?)

Every build identity element is read from the connector's `descriptor.yaml.images.<key>` map entry (per ADR-0016):

| Element | Source in descriptor.yaml |
|---|---|
| GHCR image name | `images.<key>.name` |
| Dockerfile path (relative to connector dir) | `images.<key>.dockerfile` |
| Build context (relative to connector dir) | `images.<key>.context` |
| Top-level field to patch with new tag | `images.<key>.image` (CI overwrites in place) |

No build identity lives in the workflow YAML. Renaming a GHCR image, moving a Dockerfile, or adopting a new context path is a one-line edit in the descriptor; CI follows on the next push.

## Troubleshooting

### "No second CI run after commit A"
- Open `build-images.yml` at the failing commit and confirm `src/ingestion/**` is still in `on.push.paths`. If it isn't, that's a regression — restore.
- If the workflow file looks fine, the App token may have failed silently. Check GitHub Actions tab and the App's "Recent Deliveries" for delivery errors.

### "bump-descriptors job fails on push"
- The push uses the `INSIGHT_RELEASE_APP` installation token. The App must be on `main`'s branch-protection bypass list AND have `Contents: read & write` scope on the repo.
- Look for "branch protection" or "GH001" errors in the job log.
- Same failure mode as `publish-chart`'s commit step — they share the App.

### "bump-descriptors job fails with non-semver version"
- The `Bump descriptor.version (minor) for each patched connector` step runs `.github/workflows/scripts/bump-descriptor-version.py`, which rejects values that aren't strict semver `MAJOR.MINOR.PATCH` per ADR-0015 (leading zeros like `2026.05.04`, two-segment like `1.0`, anything with a `v` prefix or pre-release suffix).
- The error message names the descriptor path and quotes the offending value. Fix the `version:` field manually in that descriptor (e.g. `version: "1.0.0"`), commit + push, and the next CI run will succeed.
- The image was already pushed to GHCR before this step ran, so re-running CI after the fix does NOT rebuild the image — `discover-images` filters by changed `context` paths and a pure version edit doesn't match any image's context. Push an empty commit (`git commit --allow-empty -m "ci: re-trigger after version fix" && git push`) to force a re-attempt with the correct flow.

### "Two umbrella publishes for one source change"
- The symptom that breaks if the `bump-descriptors.outputs.committed != 'true'` guard fails in `publish-chart`. Inspect Run 1's publish-chart `if:` expression — it MUST include that guard.
- Also check that `bump-descriptors` is in `publish-chart.needs:` (required for the output to be visible).

### "Toolbox image lacks new connector descriptor"
- Likely Run 2 didn't fire. Check `git log` on `main` — there should be a commit A from `insight-ci` between the developer's commit and the chart-release commit B. If A is missing or has `[skip ci]`, our 2-commit invariant is broken.
- Manual recovery: push an empty commit (`git commit --allow-empty -m "ci: rebuild toolbox" && git push`) to force a toolbox rebuild + chart bump.

### "Infinite descriptor-bump loop"
- This would happen if the per-connector paths-filter excludes `descriptor.yaml` were broken. Inspect the `discover` step: it MUST filter out entries whose only changed file is `descriptor.yaml`. Equivalently, the connector-level paths-filter under `changes` MUST end with `'!src/ingestion/connectors/<category>/<name>/descriptor.yaml'`.
- If you suspect a loop in progress, manually abort the running workflow from the GitHub Actions UI, then fix the filter.

### "Dev cluster not picking up new chart"
- Check `oci://ghcr.io/constructorfabric/charts/insight` listing — confirm the new semver is published.
- Check insight-gitops GitLab CI: most recent `chart-poller` run should have committed an update to `.insight-version`. If it didn't, look at the job log for skopeo/auth errors.
- Manual recovery: in insight-gitops, run the `chart-poller` job from the Pipelines UI ("Run pipeline" → manual trigger).

### "Customer install needs the new image"
- Open `environments/<env>/values.yaml` in insight-gitops. Bump `ingestion.toolboxImage` to the new tag AND `.insight-version` to the new umbrella semver. One MR; deploy manually via `make deploy ENV=<env>` from a workstation (per inventory.protected).
- Connector image refs (`images.cdk.image` / `images.enrich.image`) travel INSIDE the toolbox image — no need to pin them per-connector at the customer overlay.

## References

- [ADR-0001](../ADR/0001-chart-publishing-on-merge.md) — Chart publishing on merge.
- [ADR-0016](../../../airbyte-toolkit/specs/ADR/0016-descriptor-images-block.md) — Descriptor `images:` block as single source of truth (supersedes ADR-0011 and ADR-0014).
- [build-images.yml](../../../../../.github/workflows/build-images.yml) — the workflow itself.
- [Connector creation skill](../../../../../.cf-studio/.core/skills/connector/workflows/create.md) — Phase 3.7 documents the `images:` block + CI contract for image-bearing connectors.
