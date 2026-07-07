---
status: proposed
date: 2026-05-06
---

# GitOps Deployment — Design and Operations

This document is the technical source of truth for the hybrid GitOps deployment system used to ship the Insight platform from a public open-source codebase to internal Kubernetes clusters that sit behind a corporate VPN. It defines the data flow, the tagging contract, the manifests repository, the hourly poller, the manual deploy step, and the secret-management workflow. It is structured so that a follow-up implementation task can generate the actual repository layout and Makefile from this specification without further architectural decisions.

## Table of Contents

1. [1. Architecture Overview](#1-architecture-overview)
   - [1.1 Goals](#11-goals)
   - [1.2 Data Flow](#12-data-flow)
   - [1.3 Repositories](#13-repositories)
   - [1.4 Trust Boundaries](#14-trust-boundaries)
   - [1.5 Layer Model](#15-layer-model)
2. [2. Tagging & Versioning](#2-tagging--versioning)
   - [2.1 Image Tag Format](#21-image-tag-format)
   - [2.2 Why Date-Time-ShortSHA](#22-why-date-time-shortsha)
   - [2.3 Chart and Manifest Versioning](#23-chart-and-manifest-versioning)
   - [2.4 Chart Publishing](#24-chart-publishing)
3. [3. Step-by-Step Workflow](#3-step-by-step-workflow)
   - [3.1 Code Push to GitHub](#31-code-push-to-github)
   - [3.2 Image Build and Push to GHCR](#32-image-build-and-push-to-ghcr)
   - [3.3 GitLab Poller Updates Manifests](#33-gitlab-poller-updates-manifests)
   - [3.4 Engineer Bootstraps a Cluster (L0)](#34-engineer-bootstraps-a-cluster-l0)
   - [3.5 Engineer Provisions the System Layer (L2)](#35-engineer-provisions-the-system-layer-l2)
   - [3.6 Engineer Deploys the App (L3)](#36-engineer-deploys-the-app-l3)
4. [4. Security Implementation](#4-security-implementation)
   - [4.1 Secret Lifecycle](#41-secret-lifecycle)
   - [4.2 Passbolt Integration](#42-passbolt-integration)
   - [4.3 Sealed Secrets Sealing Flow](#43-sealed-secrets-sealing-flow)
   - [4.4 In-Repo Rules](#44-in-repo-rules)
5. [5. Local Environment Setup](#5-local-environment-setup)
   - [5.1 Required Tools](#51-required-tools)
   - [5.2 Authentication](#52-authentication)
   - [5.3 VPN and Cluster Access](#53-vpn-and-cluster-access)
6. [6. Makefile Specifications](#6-makefile-specifications)
   - [6.1 Variables and Defaults](#61-variables-and-defaults)
   - [6.2 Public Targets](#62-public-targets)
   - [6.3 Pre-flight Safety Checks](#63-pre-flight-safety-checks)
   - [6.4 Deploy Logic](#64-deploy-logic)
   - [6.5 Sealed Secret Targets](#65-sealed-secret-targets)
   - [6.6 Failure Modes](#66-failure-modes)
7. [7. Repository Layout (Target)](#7-repository-layout-target)
8. [8. Open Items](#8-open-items)

## 1. Architecture Overview

### 1.1 Goals

The deployment system has five explicit goals:

- Keep the application source code in public GitHub while keeping every byte of cluster-shaped infrastructure (Helm values, sealed secrets, environment overlays, RBAC) in the corporate GitLab behind the VPN.
- Build images in public CI and publish them to a public registry (GHCR), so external contributors can reproduce a build, but never expose cluster credentials, host names, or internal topology in any public artifact.
- Decouple "image is available" from "image is deployed" — image promotion is a routine, hands-off event; deployment is a deliberate human action with an audit trail.
- **Separate stateful infrastructure from the application.** Shared services (databases, object stores, message buses, workflow engines, data-integration platforms) live in their own namespace and have their own lifecycle, so that an app upgrade never re-rolls a database and a database upgrade never accidentally restarts the app. Each system service can also be replaced by a managed external endpoint without changing the app values surface — see [§1.5](#15-layer-model).
- Match the MVP team size: one engineer, one VPN, no shared CI runner inside the trust boundary. The Makefile-driven manual deploy is intentional for this stage and is replaceable with an in-cluster ArgoCD instance later without changing the manifests repository contract.

### 1.2 Data Flow

```
+----------------+         +---------------+        +-------------------+
|                |  push   |               |  pull  |                   |
| GitHub (OSS)   +-------->+ GitHub Actions+------->+ GHCR              |
| application    |         | build & push  |        | container images  |
| source code    |         | image w/ tag  |        | (public)          |
+----------------+         +---------------+        +---------+---------+
                                                              |
                                                              | hourly poll
                                                              v
+--------------------+   commit & push   +-------------------+--------+
|                    |<------------------+                            |
| GitLab (internal)  |                   | GitLab CI scheduled job    |
| Helm charts +      |                   | "image-poller"             |
| values + sealed    |                   | (runs once per hour)       |
| secrets            |                   +-------------------+--------+
+----------+---------+                                       |
           | git pull                                        |
           v                                                 |
+----------+---------+                                       |
|                    |  helm upgrade --install               |
| Engineer laptop    +-----+                                 |
| (VPN connected)    |     |                                 |
| make deploy        |     v                                 |
+--------------------+   +--+----------------+               |
                         |                   |               |
                         | Kubernetes (VPN)  |               |
                         | dev / stage /     |               |
                         | customers / …     |               |
                         +-------------------+               |
                                                             |
                         +-------------------+   reads       |
                         | Passbolt (corp.)  |<--------------+
                         | secret material   |  engineer
                         +-------------------+  (kubeseal time)
```

The four arrows that matter:

1. **Public outbound** — GitHub Actions push to GHCR. No secret in the egress.
2. **Public inbound** — the GitLab poller pulls chart-tag listings from `oci://ghcr.io/constructorfabric/charts/insight` via `skopeo list-tags`. No code, no secrets.
3. **Internal commit** — the poller commits an updated `image.tag` to the internal GitLab. Confined to the corporate network.
4. **Cluster apply** — the engineer's `helm upgrade --install` runs from a workstation with VPN + kubeconfig. The cluster never reaches out to GitLab; this is **pull-from-engineer**, not GitOps reconciliation.

### 1.3 Repositories

Two repositories with clearly different audiences and access policies.

| Repository             | Host | Visibility | Owns |
|------------------------|------|------------|------|
| `constructorfabric/insight`  | GitHub | public | Application source, Dockerfiles, GitHub Actions workflows that build and push images to GHCR. Open-source umbrella Helm chart definition for evaluators (no production values). |
| `infra/insight-gitops` | GitLab (internal) | private | Per-environment Helm `values.yaml`, sealed secrets, the `image-poller` GitLab CI job, the `Makefile` engineers run, and the `scripts/` it calls. Pinned chart versions, the cluster-side truth. |

Customers and external evaluators never see `infra/insight-gitops`. The public `constructorfabric/insight` ships the umbrella chart and its example values; internal production wiring (DNS, OIDC issuer, password material) lives only in the private repo.

### 1.4 Trust Boundaries

- **Public**: GitHub repo, GitHub Actions runners, GHCR. May be read by the world; signed but not confidential.
- **Corporate (VPN)**: GitLab server, GitLab CI runners, Kubernetes API endpoints, Passbolt instance, engineer workstation while the VPN is up.
- **In-cluster**: the cluster reads sealed secrets from its own etcd (after the controller decrypts) and never reaches out to either Git host.

The objects that cross from public to corporate are two read-only artifacts: **container images** (pulled by the cluster's image-pull credentials at pod-scheduling time) and the **umbrella Helm chart** at `$CHART` (pulled by the engineer's workstation at `make deploy` time per [ADR-0001](../specs/ADR/0001-chart-publishing-on-merge.md)). Both share the same supply-chain risk surface; signing/verification controls for both are a tracked follow-up — see [§8](#8-open-items). Manifests and secrets never cross outward.

### 1.5 Layer Model

Insight on Kubernetes is split into three deploy layers. Each layer has its own lifecycle, its own namespace conventions, and its own slot in the gitops repo. An upgrade of one layer never re-rolls the others.

| Layer | Purpose | Namespace | How installed | Where it lives in `infra/insight-gitops` |
|------|---------|-----------|---------------|-------------------------------------------|
| **L0 — Bootstrap** | Cluster prerequisites. Installs sealed-secrets-controller, ingress-nginx, cert-manager (and any cluster-scoped issuers/CRDs); creates the `insight-infra` and `insight` namespaces. | cluster-scoped | `make bootstrap ENV=<env>` (idempotent) | `bootstrap/<env>/` |
| **L2 — System** | Shared stateful infrastructure: MariaDB, ClickHouse, Redis, Redpanda + Redpanda Console, Airbyte, Argo Workflows. **One Helm release per service.** Each service can also be replaced by a managed external endpoint (RDS, MSK, etc.) — in which case its `system/<service>/` is simply not installed and the app values point at the external host. | `insight-infra` (one per cluster, shared by every app deploy on that cluster) | manually, deliberately. Either `cd system/<service> && helm upgrade --install …` for a values-only release, or `make system-<service> ENV=<env>` when a sealed-secret needs to be created/refreshed in the same step. | `system/<service>/` (base) + `environments/<env>/<service>-values.yaml` (per-env overlay) + `environments/<env>/sealed-secrets/insight-infra/` |
| **L3 — App** | The Insight platform itself: api-gateway, analytics, identity-resolution, frontend. The umbrella chart, app services only — no infra subcharts. | `insight` (one Insight install per cluster; ENV selects the **cluster**, not the namespace) | `make deploy-app ENV=<env>` (alias `make deploy`); pulls the umbrella chart from `oci://ghcr.io/constructorfabric/charts/insight` pinned to `.insight-version`. | `environments/<env>/values.yaml` + `environments/<env>/sealed-secrets/insight/` |

There is no L1. The numbering is reserved: cluster + node provisioning (k3s install, kubelet config, OS) sit conceptually below L0 and are out of scope for this SPEC.

#### Layer separation rules

- **L2 and L3 are independent Helm releases.** A `helm upgrade` on the umbrella chart never touches MariaDB; a MariaDB version bump never re-rolls api-gateway. Each release has its own version, its own rollback timeline, and its own engineer-approved deploy moment.
- **Cross-layer wiring uses Kubernetes DNS.** L3 app values reference L2 services by `<release>.insight-infra.svc.cluster.local`. The app's connection helpers fall back to that name when no explicit host is set in env values.
- **Managed services are first-class.** When a cluster uses RDS for MariaDB, Confluent Cloud for Redpanda, or S3 for Airbyte storage, the corresponding `system/<service>/` is simply skipped at deploy time. The app values point at the external endpoint (host + port + Secret reference). The same gitops repo describes both modes; the difference is which `make system-*` targets get run.
- **Bootstrap is the only thing that touches cluster-scoped resources.** Once L0 is in place, L2 and L3 confine themselves to their own namespaces. This makes a customer install reproducible on a managed cluster where the platform team owns L0 but Cyberfabric owns L2/L3.

#### Namespace map

For one cluster carrying environment `<env>`:

| Namespace | Owner layer | Contents |
|-----------|-------------|----------|
| `kube-system` | k8s + L0 | sealed-secrets-controller, k3s defaults |
| `ingress-nginx` | L0 | ingress-nginx controller |
| `cert-manager` | L0 | cert-manager + webhook + cainjector |
| `insight-infra` | L2 | mariadb, clickhouse, redis, redpanda, redpanda-console, airbyte, argo-workflows (each as its own Helm release) |
| `insight` | L3 | the umbrella chart (api-gateway, analytics, identity-resolution, frontend) |

Each cluster hosts exactly one Insight install. The cluster's identity (which env it represents) lives in the kube-context name (`insight-<env>`) and the gitops repo's `environments/<env>/` directory — not in the namespace. Operationally this keeps the two well-known namespace names (`insight`, `insight-infra`) the same across every install — including the local cluster (`ENV=local`) — and matches any external chart consumer's expectation of a single `insight` release.

#### Dual-purpose umbrella: `<service>.deploy` toggles

The umbrella chart in `constructorfabric/insight` keeps its infrastructure subcharts (`clickhouse`, `mariadb`, `redis`, `redpanda`) **gated by per-service `<service>.deploy: true|false` flags**. The flag selects the install shape:

| Caller | `<svc>.deploy` | Result |
|--------|----------------|--------|
| External consumer wanting a single-namespace install | `true` for all infra subcharts | Single fat Helm release in the `insight` namespace; the umbrella renders MariaDB, ClickHouse, Redis, Redpanda **and** the app services together. Self-contained, no separate L2. |
| Gitops (any cluster managed by this repo — production and local `ENV=local`) | `false` for every infra subchart | Umbrella renders the app services only, into the `insight` namespace. L2 services come from one of: (a) `make system-<service>` Helm releases in `insight-infra` per [§3.5](#35-engineer-provisions-the-system-layer-l2); (b) managed external endpoints (RDS, MSK, …); (c) a separate team's infra namespace. App values point at the actual host. |

Why dual-purpose instead of two charts:

- **One chart shape** — the local gitops cluster (`make deploy ENV=local`) exercises the same templates and layered shape that production renders. Bugs in app rendering are caught locally.
- **Customers can choose** — an external chart consumer who is fine running everything in one namespace can flip the toggles `true` and get a self-contained install. A consumer with managed infra flips them `false`.
- **No flag conflicts** — the existing `<service>.deploy: false` path already wires the app to look up `<service>.host` / `<service>.port` from values, so cross-namespace DNS (`<release>.insight-infra.svc.cluster.local`) is just one well-formed hostname away.

Airbyte and Argo Workflows are **not** subcharts of the umbrella in either mode — on every gitops cluster (production and local) they are installed as separate Helm releases via `make system-{airbyte,argo}` into `insight-infra`; no chart change required.

## 2. Tagging & Versioning

### 2.1 Image Tag Format

Every image pushed to GHCR by `constructorfabric/insight` GitHub Actions **MUST** carry exactly one canonical tag in the format:

```
YYYY.MM.DD.HH.MM-<shortSHA>
```

Worked example:

- Build started at `2026-05-06 09:17 UTC` on commit `e4e2ba36c1f9a7…`.
- shortSHA = first 7 hex characters of the commit SHA = `e4e2ba3`.
- Canonical tag = `2026.05.06.09.17-e4e2ba3`.

Rules:

- Time component is **UTC**, not workstation-local. This avoids two builds in the same minute from two time zones colliding.
- shortSHA is exactly 7 characters, lower-case, taken from `${{ github.sha }}` via `cut -c1-7`.
- Tags are immutable once pushed. A re-run on the same commit produces a tag with a later `HH.MM` and is therefore a distinct image.
- The CI step `docker push` MUST also push `:latest` for `main`-branch builds — but `:latest` is **only** consumed by ad-hoc local development, never by the GitLab manifests. The poller ignores `:latest`.

### 2.2 Why Date-Time-ShortSHA

The format is chosen to satisfy four constraints simultaneously:

| Constraint | How the format satisfies it |
|------------|-----------------------------|
| Lexicographic sort = chronological sort. | Year-major fixed-width components mean `sort -r` returns the newest tag first without parsing dates. The poller relies on this. |
| Human-readable in `kubectl describe`. | An on-call engineer reading a pod spec sees "this pod runs the 09:17 build from May 6th" without joining tables. |
| Uniquely traceable to a commit. | The shortSHA component points back to a Git revision in `constructorfabric/insight`. No name collisions in practice (7-char shortSHA collisions are rare enough on a single repo at this scale). |
| Distinguishes two builds of the same commit. | The time component changes on every CI run, so a re-run after a flaky test produces a different tag and the cluster picks up the second build, not the first. |

### 2.3 Chart and Manifest Versioning

The deployment artifact is the umbrella Helm chart, published per merge to `oci://ghcr.io/constructorfabric/charts/insight:<semver>`. The gitops repo pins exactly one published version per environment. The contract has four versioned things, each with one job:

| Field | Where | Format | Bumped by |
|---|---|---|---|
| Image tag | GHCR image, e.g. `ghcr.io/constructorfabric/insight-api-gateway:T` | `YYYY.MM.DD.HH.MM-<shortSHA>` | CI on every image build (status quo). |
| Subchart `appVersion` | `src/.../helm/Chart.yaml` (one per service) | image tag | CI, when the service's image rebuilds. |
| Subchart `version` | same `Chart.yaml` | semver | PR author, only when subchart templates change. |
| Umbrella `version` | `charts/insight/Chart.yaml` | semver, patch per publish, minor on shape change | CI per merge to `main`. |
| Umbrella `appVersion` | same `Chart.yaml` | image tag of the publishing CI run | CI per merge to `main`. Display only. |
| Gitops pin | `infra/insight-gitops/.insight-version` | one line, umbrella semver, e.g. `0.1.47` | poller (auto for `dev`); engineer MR (for any non-dev env, e.g. `stage`, `acme`, `globex`). |

Rules that follow from this:

- Each subchart's `values.yaml` defaults `image.tag = ""`, and the templates resolve via `default .Chart.AppVersion`. **Inside a subchart, `.Chart.AppVersion` is that subchart's `appVersion` — not the umbrella's.** Per-service tag granularity is preserved: rebuilding only `api-gateway` advances only that subchart's `appVersion`, while every other service stays on its prior tag.
- The umbrella's `appVersion` is the publishing CI run's build tag. It is informational only (visible in `helm list`, `kubectl describe pod`); no template reads it.
- The umbrella `version` patch-bumps per CI publish. Minor bumps require an explicit PR change to the umbrella `Chart.yaml` — used when the umbrella's own templates or values shape change.
- Image tags in environment values files are usually unset (let the chart `appVersion` flow through). Set explicitly only for hotfix scenarios — running one service at a tag different from the one bundled in the umbrella version.
- `:latest` is consumed only by ad-hoc local development, never by the chart, never by the gitops repo, never by the poller.

The `infra/insight-gitops` repository may tag its own commits as `deploy-YYYY.MM.DD.HH.MM-<shortSHA>` when an engineer runs `make tag` after a successful deploy. This is for rollback-by-checkout, not for triggering anything; the cluster's source of truth is `.insight-version` at `HEAD`.

### 2.4 Chart Publishing

Per merge to `main` of `constructorfabric/insight`, GitHub Actions publishes the umbrella chart to GHCR in one workflow. The contract — independent of any specific YAML — is:

1. Build whichever service images changed (existing behaviour).
2. For each rebuilt service, bump that subchart's `Chart.yaml` `appVersion` to the build tag. Subchart `version` is bumped only when the subchart's templates changed (PR author's call, gated by review).
3. Bump the umbrella's `Chart.yaml`: `version` patch-bumps, `appVersion` becomes the build tag.
4. Run `helm dependency update charts/insight` to regenerate `Chart.lock` from `file://` subcharts.
5. `helm package charts/insight -d dist/`.
6. `helm registry login ghcr.io` using the workflow's `GITHUB_TOKEN`.
7. `helm push dist/insight-<version>.tgz oci://ghcr.io/constructorfabric/charts`.
8. Commit the version bumps back to `main` so the repo state matches what was published.

Resulting OCI artifact: `oci://ghcr.io/constructorfabric/charts/insight:<version>`. The `charts/` segment is part of the GHCR package name (standard Helm-OCI behaviour). The decision rationale lives in [ADR 0001](../specs/ADR/0001-chart-publishing-on-merge.md).

## 3. Step-by-Step Workflow

### 3.1 Code Push to GitHub

An engineer or external contributor merges a pull request into `main` of `constructorfabric/insight`. No special tag, no release, just a merge commit.

GitHub Actions workflow `.github/workflows/build-and-push.yaml` triggers on `push: branches: [main]` and on `workflow_dispatch` for manual reruns.

### 3.2 Image Build and Push to GHCR

The workflow:

1. Checks out the commit.
2. Computes the canonical tag:
   ```bash
   export TAG="$(date -u +%Y.%m.%d.%H.%M)-$(echo "${GITHUB_SHA}" | cut -c1-7)"
   ```
3. Logs in to GHCR with the workflow's `GITHUB_TOKEN` (no PAT).
4. For each service (api-gateway, analytics, frontend, identity-resolution, …):
   - `docker buildx build --platform linux/amd64,linux/arm64 -t ghcr.io/constructorfabric/insight-<service>:${TAG} .`
   - `docker push ghcr.io/constructorfabric/insight-<service>:${TAG}`
5. Optionally pushes `:latest` for `main` builds (consumed only by local dev, never by the poller).
6. Emits the tag as a workflow output so the run page shows it without scrolling logs.

The workflow does **not** touch any private system. It does not know GitLab exists. It produces images and stops.

### 3.3 GitLab Poller Updates Manifests

A scheduled GitLab CI job `chart-poller` runs once per hour (cron `0 * * * *`) on a runner inside the corporate network. Its single responsibility: detect new umbrella chart versions on GHCR and bump the gitops pin for environments listed in `auto_envs`. It does **not** deploy.

Logic (pseudocode, implemented in `infra/insight-gitops/scripts/poller.sh`):

```
chart_repo=$(yq '.chart_repository' .poller.yaml)   # oci://ghcr.io/constructorfabric/charts/insight
pin_file=$(yq '.version_pin_file' .poller.yaml)     # .insight-version
regex=$(yq '.semver_regex' .poller.yaml)            # ^[0-9]+\.[0-9]+\.[0-9]+$

current=$(cat "$pin_file")
latest=$(skopeo list-tags docker://${chart_repo#oci://} \
         | jq -r '.Tags[]' \
         | grep -E "$regex" \
         | sort -V | tail -n1)

if [ "$current" != "$latest" ]; then
    echo "$latest" > "$pin_file"
    git add "$pin_file"
    git commit -m "chore(poller): bump .insight-version $current → $latest"
    git push origin main
fi
```

Behaviour:

- Strict semver regex on the tag listing. Pre-release tags or anything off-format is ignored.
- One commit per poll run when the pin moves; nothing committed when no new version exists.
- Commits are authored by a service account (`infra-poller@cyberfabric.local`) with a deploy key scoped to push to `main` of `infra/insight-gitops` only.
- The poller acts only on environments listed in `auto_envs` of `.poller.yaml`. `dev` is included; non-dev envs (the internal `stage`/`test` clusters and every customer-named production cluster — `acme`, `globex`, …) are **not** auto-polled, those bumps are PR'd by an engineer, see [§3.6](#36-engineer-deploys-the-app-l3).
- The poller does not write to per-environment values files. Image tags in env values are expected to be empty (the chart's per-subchart `appVersion` flows through). Hotfix-style explicit `image.tag` overrides are an engineer-authored MR, never a poller action.
- A failed `git push` (e.g. someone else pushed a manual change in the same hour) retries with `git pull --rebase` once; on a second failure it leaves the repo dirty and surfaces a CI failure.

The poller does **not** trigger a deploy. It does **not** call ArgoCD. It does **not** notify the cluster. It is a one-way reflector from GHCR to GitLab.

### 3.4 Engineer Bootstraps a Cluster (L0)

Run **once per cluster**, before any system or app deploy can proceed. Idempotent — re-running on an already-bootstrapped cluster is a no-op apart from `helm upgrade` noise.

1. **Tooling check** — `make doctor` verifies helm, kubectl, kubeseal, passbolt, skopeo, yq, gpg are present (see [§5.1](#51-required-tools)).
2. **Cluster reach** — `make doctor` also runs the equivalent of `kubectl --context insight-<env> cluster-info` so a missing VPN or wrong kubeconfig fails before anything is changed.
3. **`make bootstrap ENV=<env>`** — installs the controllers and creates the namespaces this layer is responsible for:
   - `kube-system/sealed-secrets-controller` (named so `kubeseal` finds it without flags)
   - `ingress-nginx/ingress-nginx-controller` (claims the `nginx` IngressClass)
   - `cert-manager/cert-manager` plus the per-env ClusterIssuers (`selfsigned-cluster-issuer` and `local-ca` for non-public envs; Let's Encrypt for customer-facing envs — committed under `bootstrap/<env>/issuer.yaml`)
   - the `insight-infra` namespace (shared by L2)
   - the `insight` namespace (the L3 target)
4. **Capture the sealed-secrets pub cert** — `make fetch-cert ENV=<env>` writes `environments/<env>/pub-cert.pem`, commit it. `kubeseal` uses it to encrypt secrets in subsequent steps.

After Step 4 the cluster is ready to receive L2 and L3 deploys. The layer-0 footprint is minimal (three controllers + two namespaces) and is the only part that needs cluster-admin RBAC.

### 3.5 Engineer Provisions the System Layer (L2)

Run **once per service per cluster**, then re-run only when bumping a service version or rotating its credentials. **Manual and deliberate** — there is no top-level `make deploy-system` that chains every service, because (a) most clusters do not run every service self-hosted (some swap MariaDB for RDS, some swap Redpanda for MSK), and (b) infra deploys benefit from per-service consideration in a way app deploys do not.

For each system service the engineer wants self-hosted on this cluster:

1. **(Optional) Seal the service's secrets** — for services that need a credential before the first start (e.g. MariaDB root password, Redpanda admin TLS, Airbyte object-store creds), seal them first:
   ```bash
   make seal-secret ENV=<env> NAMESPACE=insight-infra NAME=<service>-creds
   ```
   The Makefile pulls the cleartext from Passbolt resource `insight-<env>-<name>` and writes `environments/<env>/sealed-secrets/insight-infra/<service>-creds-sealedsecret.yaml`. The controller decrypts on apply.
2. **Apply the service** — two paths, depending on whether the deploy needs Makefile glue:
   - **Pure helm** — when no extra glue is required (most services), the engineer runs `helm` directly so the layer is reproducible without this repo's Makefile:
     ```bash
     helm upgrade --install <release> <chart-ref> --version <X.Y.Z> \
       --namespace insight-infra \
       -f system/<service>/values.yaml \
       -f environments/<env>/<service>-values.yaml \
       --wait --timeout 10m
     ```
   - **Makefile target** — when the deploy needs post-install glue (e.g. Airbyte's setup-wizard API call, Argo Workflows' supplemental RBAC apply), use `make system-<service> ENV=<env>` which wraps the helm step with the necessary scripting.
3. **Re-seal sealed secrets if the controller's keypair rotated** — see [§4.3](#43-sealed-secrets-sealing-flow).

The set of L2 services tracked in this repo: `mariadb`, `clickhouse`, `redis`, `redpanda`, `redpanda-console`, `airbyte`, `argo-workflows`. Each has a directory under `system/<service>/` with chart pin and base values; per-env overlays live at `environments/<env>/<service>-values.yaml`.

### 3.6 Engineer Deploys the App (L3)

The app deploy is the routine hands-off step — a deploy is always initiated by a human at a workstation, but only the app layer is auto-bumped by the poller. Steps:

1. **Sync** — `cd ~/work/insight-gitops && git pull --ff-only origin main`. The pre-flight check in [§6.3](#63-pre-flight-safety-checks) refuses to deploy if `HEAD` is not equal to `origin/main`.
2. **Inspect** — `git log --oneline origin/main ^HEAD@{upstream}` (run by `make diff`) shows the poller commits since the last deploy.
3. **VPN + context check** — `make deploy ENV=<env>` runs the cluster reachability probe and confirms the active kube-context is `insight-<env>` before doing any work.
4. **Diff** — the Makefile renders the chart with the current values (`helm template …`) and stores the rendered manifest under `.deploy/last-render-<env>.yaml`. The engineer can `diff` against the previous render to see what is changing on the cluster.
5. **Apply** — `helm upgrade --install insight $CHART --version $(cat .insight-version) -n insight -f environments/<env>/values.yaml`, where `$CHART = oci://ghcr.io/constructorfabric/charts/insight`. The chart is pulled from GHCR at deploy time; the gitops repo does **not** vendor it — see [§7](#7-repository-layout-target). The Makefile passes `--atomic --timeout 10m` so a failed deploy is rolled back automatically.
6. **Verify** — `make status ENV=<env>` runs `kubectl rollout status` for each deployment + `helm test` for smoke tests.

`make deploy` is an alias for `make deploy-app` and only touches the L3 layer. The L0 bootstrap and L2 system services are not chained — they are explicit prior steps with their own engineer-approved moments. This is by design: an app upgrade should never be able to migrate a database.

For every non-`dev` environment — both the internal `test` and `stage` clusters and every customer-named production cluster (`acme`, `globex`, …; one entry per customer install, no generic "prod"):

- The poller does not auto-bump the chart pin. An engineer opens a merge request that bumps `environments/<env>/.insight-version` (or the umbrella values file) to the desired version (typically the one currently green on `dev`).
- After review and merge, the engineer runs `make deploy ENV=<env>` from their workstation.
- For environments listed in the Makefile's `PROTECTED_ENVS` (every customer cluster; internal `test`/`stage` are at the team's discretion), `make deploy` requires an additional `CONFIRM=yes-deploy-<env>` flag — e.g. `CONFIRM=yes-deploy-acme` — so a typo on a sleepy morning does not push to a customer cluster. See [§6.2](#62-public-targets) and [§6.3](#63-pre-flight-safety-checks) for the safety check.

## 4. Security Implementation

### 4.1 Secret Lifecycle

There are three distinct states for any piece of secret material:

| State | Where it lives | How to read |
|-------|----------------|-------------|
| Raw secret | Passbolt resource named `insight-<env>-<base>` (password field carries the full cleartext Kubernetes Secret manifest **as single-line JSON** — see §4.2) | `scripts/passbolt-fetch.sh "insight-<env>-<base>"` — resolves the human name to the resource UUID via `passbolt list resource --json --filter 'Name == "…"'`, then fetches by UUID via `passbolt get resource --json --id <uuid>`. |
| Sealed manifest | `infra/insight-gitops/environments/<env>/sealed-secrets/<namespace>/<name>-sealedsecret.yaml` (committed) | Anyone with repo read access; opaque to humans |
| In-cluster Secret | Kubernetes API, decrypted by `sealed-secrets-controller` | `kubectl get secret <name> -o yaml` (RBAC-gated) |

The flow between states is one-way at write-time:

```
Passbolt ─(engineer + kubeseal)─▶ Sealed manifest ─(controller)─▶ In-cluster Secret
```

There is no path that puts a raw secret on disk in cleartext between Passbolt and the sealed manifest. The Makefile streams `scripts/passbolt-fetch.sh` (which wraps `passbolt list resource` + `passbolt get resource`) straight into `kubeseal` (see [§4.3](#43-sealed-secrets-sealing-flow)).

### 4.2 Passbolt Integration

- Authoritative store for raw passwords, OIDC client secrets, database passwords, GHCR pull secrets, TLS keys.
- **Storage convention**: one Passbolt resource per Kubernetes Secret per environment. The resource's **password field carries the entire cleartext Kubernetes Secret manifest as a single-line JSON object**, ready to be piped to `kubeseal` without further composition. JSON (not YAML) because Passbolt's password field is single-line in the UI and silently strips newlines on save; `kubeseal` accepts JSON and YAML interchangeably. The resource's URI/username/description fields are documentation only (e.g. `kubectl-namespace=insight`, `kubectl-name=insight-oidc`). Example payload (paste verbatim, with the password substituted): `{"apiVersion":"v1","kind":"Secret","metadata":{"name":"<name>","namespace":"<ns>"},"type":"Opaque","stringData":{"<key>":"<value>"}}`.
- **Naming**: `insight-<env>-<base>` (e.g. `insight-dev-oidc`, `insight-acme-db-creds`). The Makefile defaults `PASSBOLT_NAME` to this expression so the engineer rarely passes it explicitly.
- **Authentication**: each engineer's Passbolt account is bound to their personal GPG keypair. `passbolt configure` is run once per workstation to register the server URL, the user, and the private key; subsequent `passbolt get resource --json --id <uuid>` decrypts the resource via the local GPG agent (passphrase cached in the OS keychain). CI never authenticates to Passbolt — the sealing step is a human action.
- The `passbolt` CLI (community: [`go-passbolt-cli`](https://github.com/passbolt/go-passbolt-cli)) is the only sanctioned way to read a secret. Browser-extension copy/paste, screenshots, or pasting into chat are explicitly not.

### 4.3 Sealed Secrets Sealing Flow

`kubeseal` encrypts a Kubernetes `Secret` against the cluster's sealed-secrets-controller public certificate. The encrypted output is committable to Git.

The Makefile target `seal-secret` (see [§6.5](#65-sealed-secret-targets)) implements the streaming flow:

```bash
# Convention: the Passbolt resource named "insight-${ENV}-${NAME}" has,
# in its password field, the complete cleartext Kubernetes Secret
# manifest **as single-line JSON** (Passbolt strips newlines from
# multi-line text; kubeseal reads JSON too). The pipe below never
# materialises it on disk.
#
# go-passbolt-cli identifies resources by UUID, not name. The wrapper
# script `scripts/passbolt-fetch.sh` does the name → UUID resolution
# via `passbolt list resource --json --filter 'Name == "…"'` and then
# fetches by UUID via `passbolt get resource --json --id <uuid>`.
# Override the lookup with PASSBOLT_RESOURCE_ID=<uuid> when names
# collide (e.g. shared resources across folders).
scripts/passbolt-fetch.sh "insight-${ENV}-${NAME}" \
  | kubeseal --format yaml \
      --cert "environments/${ENV}/pub-cert.pem" \
  > "environments/${ENV}/sealed-secrets/${NAMESPACE}/${NAME}-sealedsecret.yaml"
```

Properties:

- The raw secret lives in the pipe only; never on disk, never in shell history. `passbolt-fetch.sh … | kubeseal …` is the canonical form.
- The Passbolt resource holds the **whole** Kubernetes Secret manifest (including `apiVersion`, `metadata.name`, `metadata.namespace`, `type`, and every key under `stringData`). `kubeseal` reads it as one object, so no `kubectl create` step is needed. Multi-key secrets (an OIDC client with seven fields) cost no more than single-key secrets.
- `pub-cert.pem` is the cluster controller's public certificate, fetched once per environment and committed to the repo at `environments/<env>/pub-cert.pem`. Renewal procedure is in §8 Open Items.
- The output file is committed. The plaintext input is not, because it never existed as a file.
- **The `${NAMESPACE}` segment in the output path is always the target namespace of the Secret manifest stored in Passbolt.** Per [§1.5](#15-layer-model) that is `insight-infra` for L2 service credentials (e.g. `mariadb-creds`, `redpanda-tls`, `airbyte-s3`) and `insight` for L3 app credentials (e.g. `insight-oidc`, `insight-db-creds`). One env directory therefore carries two namespace subdirs:

```
environments/<env>/sealed-secrets/
├── insight-infra/                      # L2 — system-layer secrets
│   ├── mariadb-creds-sealedsecret.yaml
│   ├── clickhouse-creds-sealedsecret.yaml
│   ├── redpanda-tls-sealedsecret.yaml
│   └── airbyte-s3-sealedsecret.yaml
└── insight/                            # L3 — app-layer secrets (always `insight`)
    ├── insight-oidc-sealedsecret.yaml
    └── insight-db-creds-sealedsecret.yaml
```

  The Makefile's `seal-secret` target writes whichever namespace is passed via `NAMESPACE=`; the `*-secret-template.yaml` files committed alongside each sealed manifest carry the same namespace value so `kubeseal` and the controller agree on where the decrypted Secret lands.

### 4.4 In-Repo Rules

These are non-negotiable rules enforced by review and by a pre-commit hook in `infra/insight-gitops`:

1. **No plain secrets in Git.** A pre-commit hook runs `gitleaks` against the staged diff and refuses commits that match its rule set.
2. **Sealed manifests only as `*-sealedsecret.yaml`.** Templates (which contain example/empty values) live alongside as `*-secret-template.yaml` and are explicitly listed in the hook's allowlist.
3. **Public certificate is the only key material in Git.** Private keys and Bitnami sealed-secrets-controller's master key live in the cluster only.
4. **No `.env` files.** Local development reads from `passbolt` directly (`passbolt get resource --json --id <uuid> | jq -r .Password`, or the `scripts/passbolt-fetch.sh "<name>"` wrapper), keeping the cleartext in process memory.

## 5. Local Environment Setup

### 5.1 Required Tools

The Makefile checks for each tool at the top of every target and fails fast with an installation hint when missing.

| Tool | Minimum version | Install hint (macOS) | Used for |
|------|-----------------|----------------------|----------|
| `helm` | 3.14 | `brew install helm` | Render and apply the umbrella chart. |
| `kubectl` | 1.27 | `brew install kubectl` | Cluster-side dry-runs, rollout status, log inspection. |
| `kubeseal` | 0.27 | `brew install kubeseal` | Encrypt secrets against the cluster controller public cert. |
| `passbolt` (go-passbolt-cli) | 0.7+ | `brew tap passbolt/tap && brew install go-passbolt-cli` | Read secret material from Passbolt without ever writing it to disk. |
| `gpg` | 2.4+ | `brew install gnupg` | Required by the Passbolt CLI to decrypt the user's private key locally. |
| `skopeo` | 1.14 | `brew install skopeo` | Read GHCR tag listings; also used by the GitLab poller. |
| `yq` | 4.x | `brew install yq` | Read and write `image.tag` and other values keys in YAML. |
| `git` | 2.40 | system | Repo sync. |
| `make` | GNU Make 3.81+ | system / `brew install make` | The deploy entry point. |

A `Brewfile` at the repo root captures these dependencies; `make doctor` runs `brew bundle check` and prints what is missing.

### 5.2 Authentication

- **GitHub** — read access to `constructorfabric/insight` (public) is enough for clones; pushes are protected and only the CI workflow's `GITHUB_TOKEN` can publish images.
- **GitLab** — engineer authenticates with SSH key (`gitlab.cyberfabric.internal`); the deploy-key for the poller is separate and lives only in the GitLab CI variables store.
- **GHCR** — pulls are public; the cluster's image-pull secret is only needed if the team flips an image to private later. The pull secret itself is a sealed secret in the repo.
- **Kubernetes** — engineer's kubeconfig is generated by the corporate IdP; per-cluster contexts follow `insight-<env>` (e.g. `insight-dev`, `insight-stage`, `insight-acme`). The Makefile checks `kubectl config current-context` against the requested `ENV` before any apply.
- **Passbolt** — `passbolt configure` is run once per workstation: it asks for the server URL, the user's private GPG key file, and the key passphrase. Subsequent `passbolt get resource` invocations decrypt via the local GPG agent; the passphrase is cached in the OS keychain for the agent's TTL.

### 5.3 VPN and Cluster Access

- All Kubernetes API endpoints resolve only when the corporate VPN is up. The Makefile's pre-flight runs `kubectl --request-timeout=5s cluster-info` and aborts with "VPN not connected?" if the call fails.
- DNS for `*.cyberfabric.internal` is split-horizon: workstations on VPN see internal addresses; off-VPN they see nothing. The poller's GitLab runner is permanently in-network.
- There is **no** ingress from public networks to either GitLab or the clusters. A compromise of GHCR yields only the ability to publish a malformed image, which would still need to be picked up by the poller and approved by an engineer for any non-`dev` env.

## 6. Makefile Specifications

### 6.1 Variables and Defaults

```make
# infra/insight-gitops/Makefile

# ── L3 (App) — driven by `make deploy` / `make deploy-app` ──
ENV              ?= dev
NS_APP           ?= insight-$(ENV)
RELEASE          ?= insight
CHART            ?= oci://ghcr.io/constructorfabric/charts/insight
INSIGHT_VERSION  ?= $(shell cat .insight-version)
VALUES           ?= environments/$(ENV)/values.yaml
KUBE_CTX         ?= insight-$(ENV)
TIMEOUT          ?= 10m
RENDER_DIR       := .deploy

# ── L2 (System) — shared by every `make system-*` target ──
NS_INFRA         ?= insight-infra

# Each system service has chart-ref + version pinned; values live at
# system/<service>/values.yaml with per-env overlays at
# environments/$(ENV)/<service>-values.yaml.
MARIADB_RELEASE          ?= mariadb
MARIADB_CHART            ?= oci://registry-1.docker.io/bitnamicharts/mariadb
MARIADB_VERSION          ?= 21.0.x   # pin to a specific patch in system/mariadb/

CLICKHOUSE_RELEASE       ?= clickhouse
CLICKHOUSE_CHART         ?= oci://ghcr.io/constructorfabric/charts/clickhouse
CLICKHOUSE_VERSION       ?= 0.x.y

REDIS_RELEASE            ?= redis
REDIS_CHART              ?= oci://registry-1.docker.io/bitnamicharts/redis
REDIS_VERSION            ?= 21.0.x

REDPANDA_RELEASE         ?= redpanda
REDPANDA_CHART           ?= redpanda/redpanda            # repo-style; helm repo add in target
REDPANDA_VERSION         ?= 5.x.y

REDPANDA_CONSOLE_RELEASE ?= redpanda-console
REDPANDA_CONSOLE_CHART   ?= redpanda/console
REDPANDA_CONSOLE_VERSION ?= 1.x.y

AIRBYTE_RELEASE          ?= airbyte
AIRBYTE_CHART            ?= airbyte/airbyte
AIRBYTE_VERSION          ?= 1.8.5

ARGO_RELEASE             ?= argo-workflows
ARGO_CHART               ?= argo/argo-workflows
ARGO_VERSION             ?= 0.45.16

# ── L0 (Bootstrap) — see §3.4 ──
INGRESS_NGINX_VERSION    ?= 4.13.0
CERT_MANAGER_VERSION     ?= v1.18.0
SEALED_SECRETS_VERSION   ?= 2.17.4
```

- All variables are overridable on the command line (`make deploy ENV=stage`, `make system-airbyte ENV=acme AIRBYTE_VERSION=1.9.0`).
- `ENV` selects which values file, which kube-context, which sealed-secrets directory, and which app namespace.
- `NS_APP` (= `insight-$(ENV)`) is the L3 target namespace; `NS_INFRA` (= `insight-infra`, cluster-shared) is the L2 target namespace. Both are created by `make bootstrap` (see [§3.4](#34-engineer-bootstraps-a-cluster-l0)).
- `INSIGHT_VERSION` defaults to the contents of `.insight-version` at the repo root — the umbrella semver currently pinned for this repo. Override only for ad-hoc one-off renders (`make diff INSIGHT_VERSION=0.1.42`).
- System service versions are pinned per service; bumping any of them is an explicit edit, never automatic.
- The `.deploy/` directory is `.gitignore`d and stores the last rendered manifest plus a per-deploy log file.

### 6.2 Public Targets

Targets are grouped by the layer they affect (see [§1.5](#15-layer-model)). L0 and L2 are deliberate, hands-on; L3 is the routine deploy.

**Layer-agnostic**

| Target | Purpose | Pre-flight | Effect |
|--------|---------|------------|--------|
| `make doctor` | Verify required tooling, auth, and cluster reach. | none | Read-only checks; prints status. |
| `make sync` | `git fetch && git pull --ff-only origin main`. | none | Updates local repo to match `origin/main`. |
| `make tag` | Tag the deploy commit as `deploy-…`. | `sync-clean` | Local + remote git tag. Optional. |

**L0 — Bootstrap (cluster prereqs + namespaces)**

| Target | Purpose | Pre-flight | Effect |
|--------|---------|------------|--------|
| `make bootstrap ENV=<env>` | Install ingress-nginx + cert-manager + sealed-secrets-controller; create `insight-infra` + `insight` namespaces; apply per-env ClusterIssuers from `bootstrap/<env>/`. | `kube-ctx` | Three Helm releases + namespace creation + ClusterIssuer apply. Idempotent. |
| `make bootstrap-status ENV=<env>` | Show what L0 has installed on this cluster. | `kube-ctx` | Read-only. |
| `make fetch-cert ENV=<env>` | `kubeseal --fetch-cert > environments/<env>/pub-cert.pem`. | `kube-ctx` | Writes one file. |

**L2 — System (shared infrastructure)**

| Target | Purpose | Pre-flight | Effect |
|--------|---------|------------|--------|
| `make system-mariadb ENV=<env>` | `helm upgrade --install` MariaDB into `insight-infra`. | `vpn-up`, `kube-ctx` | One Helm release in `insight-infra`. |
| `make system-clickhouse ENV=<env>` | Same for ClickHouse. | `vpn-up`, `kube-ctx` | … |
| `make system-redis ENV=<env>` | Same for Redis. | `vpn-up`, `kube-ctx` | … |
| `make system-redpanda ENV=<env>` | Same for Redpanda (broker). | `vpn-up`, `kube-ctx` | … |
| `make system-redpanda-console ENV=<env>` | Redpanda Console UI as a separate release. | `vpn-up`, `kube-ctx` | … |
| `make system-airbyte ENV=<env>` | Helm-install Airbyte and run the post-install setup-wizard via `scripts/airbyte-setup.sh`. | `vpn-up`, `kube-ctx` | One Helm release + one API call. |
| `make system-argo ENV=<env>` | Helm-install Argo Workflows + apply the supplemental RBAC template (`bootstrap/argo-rbac.yaml.tmpl` substituted via `envsubst`). | `vpn-up`, `kube-ctx` | One Helm release + one Role+RoleBinding. |
| `make system-status ENV=<env>` | List Helm releases in `insight-infra` and pod readiness. | `vpn-up`, `kube-ctx` | Read-only. |

There is **no top-level `make system`** that chains every L2 target. Each cluster picks which services it self-hosts vs. swaps for managed external endpoints; chaining everything would force a self-hosted choice on operators who wanted RDS or MSK. Engineers run the subset they need.

**L3 — App (the Insight umbrella chart)**

| Target | Purpose | Pre-flight | Effect |
|--------|---------|------------|--------|
| `make diff ENV=<env>` | Show poller commits since last deploy and rendered-manifest diff. | `sync-clean` | Read-only. |
| `make deploy ENV=<env>` (alias for `deploy-app`) | Apply the umbrella chart to the `insight` namespace on this env's cluster. | `sync-clean`, `vpn-up`, `kube-ctx`, `confirm` (only fires for envs in `PROTECTED_ENVS`), `chart-present` | `helm upgrade --install --atomic`. |
| `make rollback ENV=<env>` | Roll back the umbrella to the previous Helm revision. | `vpn-up`, `kube-ctx` | `helm rollback`. |
| `make status ENV=<env>` | Show app release status and rollout health. | `vpn-up`, `kube-ctx` | Read-only. |

**Sealed secrets** (used by L2 and L3 alike — see [§4.3](#43-sealed-secrets-sealing-flow))

| Target | Purpose | Pre-flight | Effect |
|--------|---------|------------|--------|
| `make seal-secret ENV=<env> NAMESPACE=<ns> NAME=<name> [PASSBOLT_NAME=…]` | Seal the cleartext Secret YAML stored in Passbolt into a sealed-secret manifest. `NAMESPACE` is `insight-infra` for L2 secrets and `insight` for L3. `PASSBOLT_NAME` defaults to `insight-$(ENV)-$(NAME)`. | `passbolt-configured` | Writes one `*-sealedsecret.yaml`. |
| `make clear-seal-template NAME=… NAMESPACE=…` | Reset a template file to empty values. | none | Edits the template in place. |

### 6.3 Pre-flight Safety Checks

Each check is a phony target so it composes:

```make
.PHONY: sync-clean
sync-clean:
	@if [ -n "$$(git status --porcelain)" ]; then \
		echo "ERROR: working tree has uncommitted changes"; exit 1; fi
	@git fetch --quiet origin
	@LOCAL=$$(git rev-parse @); REMOTE=$$(git rev-parse @{u}); \
	if [ "$$LOCAL" != "$$REMOTE" ]; then \
		echo "ERROR: HEAD is not equal to origin/main; run 'make sync' first"; exit 1; fi

.PHONY: vpn-up
vpn-up:
	@kubectl --context $(KUBE_CTX) --request-timeout=5s cluster-info >/dev/null 2>&1 \
		|| { echo "ERROR: cannot reach cluster; is the VPN up?"; exit 1; }

.PHONY: kube-ctx
kube-ctx:
	@CUR=$$(kubectl config current-context); \
	if [ "$$CUR" != "$(KUBE_CTX)" ]; then \
		echo "ERROR: current kube-context is '$$CUR', expected '$(KUBE_CTX)'"; \
		echo "       run: kubectl config use-context $(KUBE_CTX)"; \
		exit 1; fi

# PROTECTED_ENVS is the list of customer-named production clusters
# (and any internal env the team wants gated). Add new entries as
# customers come online — acme, globex, …
PROTECTED_ENVS := acme

.PHONY: confirm
confirm:
	@if echo " $(PROTECTED_ENVS) " | grep -q " $(ENV) "; then \
		EXPECTED="yes-deploy-$(ENV)"; \
		if [ "$(CONFIRM)" != "$$EXPECTED" ]; then \
			echo "ERROR: deploy to '$(ENV)' requires CONFIRM=$$EXPECTED"; \
			exit 1; fi; \
	fi

.PHONY: passbolt-configured
passbolt-configured:
	@passbolt verify >/dev/null 2>&1 \
		|| { echo "ERROR: Passbolt CLI not configured or unreachable; run 'passbolt configure' (and connect VPN if your Passbolt is internal)"; exit 1; }
```

Rationale, one line per check:

- `sync-clean` rejects ambiguous state. The cluster must reflect a known commit.
- `vpn-up` makes "wrong network" a clean error rather than a 10-minute Helm timeout.
- `kube-ctx` prevents the worst class of accident: deploying customer values into the wrong cluster (or `dev` values into a customer cluster) because the context was left selected from a previous task.
- `confirm` is a deliberately ugly flag, scoped per env. If you can type `CONFIRM=yes-deploy-acme`, you have looked at it. Each customer cluster requires its own token (`yes-deploy-globex`, …) so muscle memory does not carry across customers.
- `passbolt-configured` is checked at the start of `seal-secret` rather than inside the pipe so the failure message is clear.

### 6.4 Deploy Logic

#### L3 — App deploy

```make
.PHONY: deploy
deploy: deploy-app

.PHONY: deploy-app
deploy-app: sync-clean vpn-up kube-ctx confirm chart-present
	@mkdir -p $(RENDER_DIR)
	@helm template $(RELEASE) $(CHART) --version $(INSIGHT_VERSION) \
		-n $(NS_APP) -f $(VALUES) \
		> $(RENDER_DIR)/last-render-$(ENV).yaml
	@echo "Rendered $(CHART):$(INSIGHT_VERSION) to $(RENDER_DIR)/last-render-$(ENV).yaml"
	helm upgrade --install $(RELEASE) $(CHART) \
		--version $(INSIGHT_VERSION) \
		--namespace $(NS_APP) --create-namespace \
		-f $(VALUES) \
		--atomic --timeout $(TIMEOUT) \
		--history-max 10 \
		| tee $(RENDER_DIR)/deploy-$(ENV)-$$(date -u +%Y%m%d-%H%M%S).log

.PHONY: chart-present
chart-present:
	@helm show chart $(CHART) --version $(INSIGHT_VERSION) >/dev/null 2>&1 \
		|| { echo "ERROR: chart $(CHART):$(INSIGHT_VERSION) not found in registry"; exit 1; }
```

Notes:

- `$(CHART)` resolves to `oci://ghcr.io/constructorfabric/charts/insight`; `$(INSIGHT_VERSION)` resolves to the contents of `.insight-version`. Together they uniquely identify the published artifact.
- `$(NS_APP)` is `insight-$(ENV)` per [§1.5](#15-layer-model). `--create-namespace` makes `make deploy` runnable on a freshly bootstrapped cluster even before the engineer has explicitly created the namespace.
- `chart-present` fails fast if the OCI tag does not exist — saves the engineer a confusing `helm upgrade` error.
- `helm template` first lets the engineer abort if the rendered diff looks wrong.
- `--atomic` rolls back on failure; combined with `--timeout 10m` it bounds blast radius.
- `--history-max 10` keeps Helm's internal release history bounded so `make rollback` always has a target.
- `make deploy` does **not** chain L2 system deploys. Each `system-*` target is a deliberate, separate engineer action — see §6.4's L2 block below and the rationale in [§3.5](#35-engineer-provisions-the-system-layer-l2).

#### L2 — System service deploys

Every system service follows the same shape, parameterised by service name. The skeleton (one example shown; the rest are mechanically identical):

```make
# system-airbyte: Helm-install Airbyte into insight-infra and run the
# post-install setup-wizard via scripts/airbyte-setup.sh.
.PHONY: system-airbyte
system-airbyte: vpn-up kube-ctx
	@helm repo add airbyte https://airbytehq.github.io/helm-charts >/dev/null 2>&1 || true
	@helm repo update airbyte >/dev/null
	@VALUES_ARGS="-f system/airbyte/values.yaml"; \
	if [ -s environments/$(ENV)/airbyte-values.yaml ]; then \
		VALUES_ARGS="$$VALUES_ARGS -f environments/$(ENV)/airbyte-values.yaml"; \
	fi; \
	helm upgrade --install $(AIRBYTE_RELEASE) $(AIRBYTE_CHART) \
		--namespace $(NS_INFRA) --create-namespace \
		--version $(AIRBYTE_VERSION) \
		$$VALUES_ARGS \
		--wait --timeout 15m
	@NAMESPACE=$(NS_INFRA) AIRBYTE_RELEASE=$(AIRBYTE_RELEASE) \
		bash scripts/airbyte-setup.sh
```

The MariaDB / ClickHouse / Redis / Redpanda / Redpanda-Console targets are the same shape minus the post-install setup script. The Argo Workflows target additionally applies the templated RBAC at `bootstrap/argo-rbac.yaml.tmpl` via `envsubst`.

Notes:

- Each `system-*` target is **idempotent**: re-running on an unchanged cluster is a no-op apart from helm-upgrade noise.
- Per-env overlays (`environments/$(ENV)/<service>-values.yaml`) are conditionally loaded — empty/missing overlay falls back to `system/<service>/values.yaml` defaults.
- For services that need a sealed secret in place before the first start (e.g. MariaDB root password), engineers run `make seal-secret NAMESPACE=insight-infra NAME=<service>-creds ENV=<env>` first; the resulting `*-sealedsecret.yaml` is committed and applied alongside the Helm release.
- A service that's swapped for a managed external endpoint is simply not invoked: the engineer skips the `system-<service>` step on that cluster, and the L3 app values point at the external host instead.

### 6.5 Sealed Secret Targets

```make
.PHONY: seal-secret
seal-secret: passbolt-configured
	@test -n "$(NAME)"      || { echo "NAME is required"; exit 1; }
	@test -n "$(NAMESPACE)" || { echo "NAMESPACE is required"; exit 1; }
	@PB_NAME="$${PASSBOLT_NAME:-insight-$(ENV)-$(NAME)}"; \
	mkdir -p environments/$(ENV)/sealed-secrets/$(NAMESPACE); \
	scripts/passbolt-fetch.sh "$$PB_NAME" \
	  | kubeseal --format yaml \
	      --cert "environments/$(ENV)/pub-cert.pem" \
	  > "environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-sealedsecret.yaml"
	@echo "Wrote environments/$(ENV)/sealed-secrets/$(NAMESPACE)/$(NAME)-sealedsecret.yaml"
```

`clear-seal-template` is shaped like the helper in `apps-gitops/Makefile` and is used to sanitise an existing template before re-sealing.

### 6.6 Failure Modes

| Failure | Where it surfaces | Action |
|---------|-------------------|--------|
| Working tree dirty. | `sync-clean`. | Engineer commits or stashes. |
| HEAD behind `origin/main`. | `sync-clean`. | `make sync`. |
| VPN down. | `vpn-up`. | Connect, retry. |
| Wrong kube-context. | `kube-ctx`. | `kubectl config use-context …`, retry. |
| Helm timeout / rollback. | `helm upgrade --atomic`. | `make status` shows rollback; `make rollback` is a no-op afterwards. |
| Poller commit reverted upstream. | `make diff`. | Engineer inspects commits; if a tag is wrong, opens an MR to reset it. |
| Sealed-secret controller cert rotation. | `kubeseal` succeeds locally but the pod fails to decrypt. | Re-pull `pub-cert.pem`, re-seal all secrets, MR. See §8. |
| Concurrent engineer deploy. | `helm upgrade` returns "another operation in progress". | Wait, retry; first deploy wins. |

## 7. Repository Layout (Target)

The implementation phase materialises this layout in `infra/insight-gitops`. Three top-level directories carry the three deploy layers:

- `bootstrap/` — L0 prereqs and per-env ClusterIssuers (cluster-scoped, runs once per cluster)
- `system/` — L2 base values + chart pins for self-hosted infrastructure (per-service directories)
- `environments/` — L3 app overlays + per-env L2 overlays + per-env sealed secrets (one directory per env)

```
infra/insight-gitops/
├── Brewfile
├── Makefile
├── README.md
├── .insight-version            # L3 — one-line umbrella semver pin (e.g. 0.1.47)
├── .poller.yaml                # chart_repository + version_pin_file + auto_envs
├── .gitlab-ci.yml              # the hourly chart-poller scheduled job
├── .gitleaks.toml              # secret-scanning rules
│
├── bootstrap/                  # ── L0 — cluster prereqs + namespaces ──
│   ├── argo-rbac.yaml.tmpl     # supplemental Argo RBAC, applied by `make system-argo`
│   └── <env>/
│       ├── ingress-nginx-values.yaml
│       ├── cert-manager-values.yaml
│       ├── sealed-secrets-values.yaml
│       └── issuer.yaml         # ClusterIssuers — selfsigned + local-ca for non-public
│                               # envs; Let's Encrypt for customer-facing envs.
│
├── system/                     # ── L2 — shared infrastructure (cluster-shared base) ──
│   ├── mariadb/
│   │   └── values.yaml
│   ├── clickhouse/
│   │   └── values.yaml
│   ├── redis/
│   │   └── values.yaml
│   ├── redpanda/
│   │   └── values.yaml
│   ├── redpanda-console/
│   │   └── values.yaml
│   ├── airbyte/
│   │   └── values.yaml
│   └── argo-workflows/
│       └── values.yaml
│
├── environments/               # ── L3 + per-env L2 overlays + per-env secrets ──
│   ├── dev/                       # internal — auto-bumped by the poller
│   │   ├── values.yaml            # umbrella app overlay (L3)
│   │   ├── mariadb-values.yaml    # per-env L2 overlay (optional; layered on system/mariadb/values.yaml)
│   │   ├── clickhouse-values.yaml # …
│   │   ├── redis-values.yaml
│   │   ├── redpanda-values.yaml
│   │   ├── redpanda-console-values.yaml
│   │   ├── airbyte-values.yaml
│   │   ├── argo-workflows-values.yaml
│   │   ├── pub-cert.pem           # sealed-secrets-controller public cert for THIS cluster
│   │   └── sealed-secrets/
│   │       ├── insight-infra/     # L2 secrets (target namespace = insight-infra)
│   │       │   ├── mariadb-creds-sealedsecret.yaml
│   │       │   ├── clickhouse-creds-sealedsecret.yaml
│   │       │   ├── redpanda-tls-sealedsecret.yaml
│   │       │   └── airbyte-s3-sealedsecret.yaml
│   │       └── insight/           # L3 secrets (target namespace = insight, always)
│   │           ├── insight-oidc-sealedsecret.yaml
│   │           └── insight-db-creds-sealedsecret.yaml
│   ├── stage/                     # internal — promote-by-MR (optional)
│   │   └── …                      # same shape as dev/
│   ├── acme/                      # customer prod — promote-by-MR + confirm token
│   │   └── …                      # same shape; sealed-secrets/insight/ for L3
│   └── <other-customer>/          # one dir per customer install (globex, …)
│       └── …                      # same shape
│
└── scripts/
    ├── poller.sh                  # invoked by .gitlab-ci.yml on cron (bumps .insight-version only)
    ├── doctor.sh                  # invoked by `make doctor`
    ├── render-diff.sh             # invoked by `make diff`
    ├── passbolt-fetch.sh          # streams cleartext Secret YAML from Passbolt
    └── airbyte-setup.sh           # post-install Airbyte setup-wizard automation
```

Conventions:

- **L0 files** in `bootstrap/<env>/`. Per-env because cert-manager issuers (Let's Encrypt prod vs. selfsigned local) genuinely differ by env. Cluster-scoped resources (ingress-nginx, sealed-secrets-controller) reuse the same values across envs by referring to chart defaults; per-env overrides are an as-needed addition.
- **L2 files** are split by service (`system/<service>/values.yaml`) to keep each release independently readable. Per-env tuning lives in `environments/<env>/<service>-values.yaml` and is layered on top at deploy time. A self-hosted cluster carries the values; a managed-external cluster simply does not run that `make system-<service>` target.
- **L3 files** stay where they were: one `values.yaml` per env for the umbrella chart, plus one `pub-cert.pem` per cluster.
- **Sealed secrets** are split by **target namespace** under `environments/<env>/sealed-secrets/<namespace>/`. `insight-infra` for L2, `insight` for L3 — the two well-known namespaces every cluster has. The `*-secret-template.yaml` template files live next to their sealed counterparts and carry the same target namespace.
- **The umbrella chart lives in `constructorfabric/insight` and is published per merge to `oci://ghcr.io/constructorfabric/charts/insight`.** The gitops repo is settings-only — values, sealed secrets, the Makefile, the poller. It does **not** vendor the chart and does **not** require a local checkout of `constructorfabric/insight`. The Makefile pulls the chart from OCI at deploy time, pinned to the version in `.insight-version`.
- **System chart pins** live in the Makefile as variables (`MARIADB_VERSION`, `CLICKHOUSE_VERSION`, …) so a bump is a single deliberate edit.

## 8. Open Items

These are accepted gaps that do not block the MVP but must be tracked.

- **Public certificate rotation.** The sealed-secrets-controller rotates its keypair periodically; when it does, the committed `pub-cert.pem` files go stale and previously sealed secrets continue to decrypt (old keys are kept), but new ones must be sealed against the new cert. Procedure: `kubeseal --fetch-cert > environments/<env>/pub-cert.pem`, commit, re-seal any in-flight changes. A scheduled monthly check is appropriate; not yet automated.
- **Promotion-MR poller for non-`dev` envs.** Currently only `dev` is auto-bumped. For internal `stage`/`test` and every customer-named cluster (`acme`, `globex`, …), the team may want a "dry-run" poller that opens a merge request rather than committing to `main`. Captured but not yet designed.
- **Migration to in-cluster ArgoCD.** The Makefile-driven manual deploy is an MVP shortcut. Once a managed ArgoCD instance is provisioned inside the corporate network, the same `infra/insight-gitops` repo becomes its source. The contract (one `values.yaml` per environment, sealed secrets per namespace) is designed to survive that migration unchanged; only the trigger mechanism changes from `make deploy` to ArgoCD reconciliation.
- **Artifact signing (images + chart).** Neither GHCR images nor the umbrella Helm chart at `oci://ghcr.io/constructorfabric/charts/insight` are signed today. The deploy admits any image tag the poller resolves and any chart version `.insight-version` pins. Follow-up: cosign-sign both at publish time, have `make chart-present` verify the chart signature before allowing deploy, and add `cosign verify` to the cluster admission policy for images.
- **Audit log of deploys.** `make deploy` writes a local log file; there is no central audit. A trivial follow-up posts the log to a `#deploys` Slack channel via the poller's bot token; deferred until the team needs it.
- **Rollback-by-tag.** `make rollback` calls `helm rollback` to the previous revision. Rolling back to an arbitrary historical state is `git checkout <deploy-tag> && make deploy`, which works but has not been rehearsed.
- **Cross-namespace defaults in the umbrella.** The umbrella keeps its infra subcharts gated by `<service>.deploy: true|false` (see [§1.5 dual-purpose umbrella](#15-layer-model)). For the gitops production case (`.deploy: false`), the app's connection helpers must default the host to `<release>.insight-infra.svc.cluster.local` when no explicit `<service>.host` is supplied — so a values file that only says `<service>.deploy: false` "just works" against `insight-infra`. Verify the helpers do this; if not, a small chart-template change is needed. Also document the dual-purpose intent in `charts/insight/README.md` so external chart consumers understand the toggle.
- **Airbyte/Argo namespace assumptions.** On every gitops cluster (production and local `ENV=local`) Airbyte and Argo Workflows run as L2 releases in `insight-infra`, while the umbrella runs in `insight`. The chart's helpers must therefore resolve the Airbyte API URL and Argo SA name across namespaces — confirm by render. If anything still hard-codes the `insight` namespace for these in templates, parameterise it.
- **L2 chart-pin policy.** System service chart versions (`MARIADB_VERSION`, `CLICKHOUSE_VERSION`, etc.) are Makefile constants today; bumping is a deliberate PR. A future enhancement: split each service's pin into its own `system/<service>/.version` file (mirroring `.insight-version`) so a poller could pre-flight version compatibility against published Bitnami / Redpanda / Airbyte releases. Out of scope for v0.
- **Per-cluster L2 inventory.** When a cluster swaps a self-hosted service for a managed endpoint (e.g. a managed database instead of `system/mariadb`), the gitops repo currently has no machine-readable record of "this cluster runs MariaDB on-cluster vs. external." A small `environments/<env>/inventory.yaml` listing which `system-*` targets to run on this cluster would make `make doctor` able to validate that the cluster matches the expected inventory, and would make it possible to render a per-customer install runbook from the repo. Captured.

- **Per-service database provisioning works only when `mariadb.deploy=true`.** The umbrella chart's `templates/mariadb-initdb-scripts.yaml` ConfigMap creates per-service databases (currently `identity` per ADR-0006; future services as they land) via Bitnami's `primary.initdbScriptsConfigMap` — which the bundled MariaDB only runs on **first pod boot**. In the gitops layered model (`mariadb.deploy=false` + external L2 MariaDB), the ConfigMap is not emitted at all, leaving the gitops repo with a choice between (a) mirroring the chart's per-service DB list into `system/mariadb/values.yaml` — couples gitops to chart, breaks every time a new per-service DB lands in the chart — or (b) the engineer creates each DB manually. Neither is good. **Proper fix**: emit a Helm pre-install/pre-upgrade hook Job from the umbrella that runs against `mariadb.host:mariadb.port` (bundled or external) using `mariadb-root-password` from `insight-db-creds`, executing `CREATE DATABASE IF NOT EXISTS` + `GRANT` for every per-service DB the chart's values declare. Engineers in gitops mode pre-create `insight-db-creds` with `mariadb-root-password` included (currently they only seal `mariadb-password` + `clickhouse-password` + `redis-password`). The chart's previous `identity-db-init-job.yaml` had this shape and was rolled back over a credential-timing bug with a regular-template Secret — fix the hook ordering and bring it back, parameterised on the per-service DB list.
