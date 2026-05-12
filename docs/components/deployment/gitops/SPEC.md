---
status: proposed
date: 2026-05-06
---

# SPEC — Hybrid GitOps Deployment

This document is the technical source of truth for the hybrid GitOps deployment system used to ship the Insight platform from a public open-source codebase to internal Kubernetes clusters that sit behind a corporate VPN. It defines the data flow, the tagging contract, the manifests repository, the hourly poller, the manual deploy step, and the secret-management workflow. It is structured so that a follow-up implementation task can generate the actual repository layout and Makefile from this specification without further architectural decisions.

## Table of Contents

1. [1. Architecture Overview](#1-architecture-overview)
   - [1.1 Goals](#11-goals)
   - [1.2 Data Flow](#12-data-flow)
   - [1.3 Repositories](#13-repositories)
   - [1.4 Trust Boundaries](#14-trust-boundaries)
2. [2. Tagging & Versioning](#2-tagging--versioning)
   - [2.1 Image Tag Format](#21-image-tag-format)
   - [2.2 Why Date-Time-ShortSHA](#22-why-date-time-shortsha)
   - [2.3 Chart and Manifest Versioning](#23-chart-and-manifest-versioning)
   - [2.4 Chart Publishing](#24-chart-publishing)
3. [3. Step-by-Step Workflow](#3-step-by-step-workflow)
   - [3.1 Code Push to GitHub](#31-code-push-to-github)
   - [3.2 Image Build and Push to GHCR](#32-image-build-and-push-to-ghcr)
   - [3.3 GitLab Poller Updates Manifests](#33-gitlab-poller-updates-manifests)
   - [3.4 Engineer Pulls and Deploys](#34-engineer-pulls-and-deploys)
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

The deployment system has four explicit goals:

- Keep the application source code in public GitHub while keeping every byte of cluster-shaped infrastructure (Helm values, sealed secrets, environment overlays, RBAC) in the corporate GitLab behind the VPN.
- Build images in public CI and publish them to a public registry (GHCR), so external contributors can reproduce a build, but never expose cluster credentials, host names, or internal topology in any public artifact.
- Decouple "image is available" from "image is deployed" — image promotion is a routine, hands-off event; deployment is a deliberate human action with an audit trail.
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
                         | virtuozzo / …     |               |
                         +-------------------+               |
                                                             |
                         +-------------------+   reads       |
                         | Passbolt (corp.)  |<--------------+
                         | secret material   |  engineer
                         +-------------------+  (kubeseal time)
```

The four arrows that matter:

1. **Public outbound** — GitHub Actions push to GHCR. No secret in the egress.
2. **Public inbound** — the GitLab poller pulls chart-tag listings from `oci://ghcr.io/cyberfabric/charts/insight` via `skopeo list-tags`. No code, no secrets.
3. **Internal commit** — the poller commits an updated `image.tag` to the internal GitLab. Confined to the corporate network.
4. **Cluster apply** — the engineer's `helm upgrade --install` runs from a workstation with VPN + kubeconfig. The cluster never reaches out to GitLab; this is **pull-from-engineer**, not GitOps reconciliation.

### 1.3 Repositories

Two repositories with clearly different audiences and access policies.

| Repository             | Host | Visibility | Owns |
|------------------------|------|------------|------|
| `cyberfabric/insight`  | GitHub | public | Application source, Dockerfiles, GitHub Actions workflows that build and push images to GHCR. Open-source umbrella Helm chart definition for evaluators (no production values). |
| `infra/insight-gitops` | GitLab (internal) | private | Per-environment Helm `values.yaml`, sealed secrets, the `image-poller` GitLab CI job, the `Makefile` engineers run, and the `scripts/` it calls. Pinned chart versions, the cluster-side truth. |

Customers and external evaluators never see `infra/insight-gitops`. The public `cyberfabric/insight` ships the umbrella chart and its example values; internal production wiring (DNS, OIDC issuer, password material) lives only in the private repo.

### 1.4 Trust Boundaries

- **Public**: GitHub repo, GitHub Actions runners, GHCR. May be read by the world; signed but not confidential.
- **Corporate (VPN)**: GitLab server, GitLab CI runners, Kubernetes API endpoints, Passbolt instance, engineer workstation while the VPN is up.
- **In-cluster**: the cluster reads sealed secrets from its own etcd (after the controller decrypts) and never reaches out to either Git host.

The objects that cross from public to corporate are two read-only artifacts: **container images** (pulled by the cluster's image-pull credentials at pod-scheduling time) and the **umbrella Helm chart** at `$CHART` (pulled by the engineer's workstation at `make deploy` time per [ADR-0001](../specs/ADR/0001-chart-publishing-on-merge.md)). Both share the same supply-chain risk surface; signing/verification controls for both are a tracked follow-up — see [§8](#8-open-items). Manifests and secrets never cross outward.

## 2. Tagging & Versioning

### 2.1 Image Tag Format

Every image pushed to GHCR by `cyberfabric/insight` GitHub Actions **MUST** carry exactly one canonical tag in the format:

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
| Uniquely traceable to a commit. | The shortSHA component points back to a Git revision in `cyberfabric/insight`. No name collisions in practice (7-char shortSHA collisions are rare enough on a single repo at this scale). |
| Distinguishes two builds of the same commit. | The time component changes on every CI run, so a re-run after a flaky test produces a different tag and the cluster picks up the second build, not the first. |

### 2.3 Chart and Manifest Versioning

The deployment artifact is the umbrella Helm chart, published per merge to `oci://ghcr.io/cyberfabric/charts/insight:<semver>`. The gitops repo pins exactly one published version per environment. The contract has four versioned things, each with one job:

| Field | Where | Format | Bumped by |
|---|---|---|---|
| Image tag | GHCR image, e.g. `ghcr.io/cyberfabric/insight-api-gateway:T` | `YYYY.MM.DD.HH.MM-<shortSHA>` | CI on every image build (status quo). |
| Subchart `appVersion` | `src/.../helm/Chart.yaml` (one per service) | image tag | CI, when the service's image rebuilds. |
| Subchart `version` | same `Chart.yaml` | semver | PR author, only when subchart templates change. |
| Umbrella `version` | `charts/insight/Chart.yaml` | semver, patch per publish, minor on shape change | CI per merge to `main`. |
| Umbrella `appVersion` | same `Chart.yaml` | image tag of the publishing CI run | CI per merge to `main`. Display only. |
| Gitops pin | `infra/insight-gitops/.insight-version` | one line, umbrella semver, e.g. `0.1.47` | poller (auto for `dev`); engineer MR (for any non-dev env, e.g. `stage`, `virtuozzo`, `constructor`, `acronis`). |

Rules that follow from this:

- Each subchart's `values.yaml` defaults `image.tag = ""`, and the templates resolve via `default .Chart.AppVersion`. **Inside a subchart, `.Chart.AppVersion` is that subchart's `appVersion` — not the umbrella's.** Per-service tag granularity is preserved: rebuilding only `api-gateway` advances only that subchart's `appVersion`, while every other service stays on its prior tag.
- The umbrella's `appVersion` is the publishing CI run's build tag. It is informational only (visible in `helm list`, `kubectl describe pod`); no template reads it.
- The umbrella `version` patch-bumps per CI publish. Minor bumps require an explicit PR change to the umbrella `Chart.yaml` — used when the umbrella's own templates or values shape change.
- Image tags in environment values files are usually unset (let the chart `appVersion` flow through). Set explicitly only for hotfix scenarios — running one service at a tag different from the one bundled in the umbrella version.
- `:latest` is consumed only by ad-hoc local development, never by the chart, never by the gitops repo, never by the poller.

The `infra/insight-gitops` repository may tag its own commits as `deploy-YYYY.MM.DD.HH.MM-<shortSHA>` when an engineer runs `make tag` after a successful deploy. This is for rollback-by-checkout, not for triggering anything; the cluster's source of truth is `.insight-version` at `HEAD`.

### 2.4 Chart Publishing

Per merge to `main` of `cyberfabric/insight`, GitHub Actions publishes the umbrella chart to GHCR in one workflow. The contract — independent of any specific YAML — is:

1. Build whichever service images changed (existing behaviour).
2. For each rebuilt service, bump that subchart's `Chart.yaml` `appVersion` to the build tag. Subchart `version` is bumped only when the subchart's templates changed (PR author's call, gated by review).
3. Bump the umbrella's `Chart.yaml`: `version` patch-bumps, `appVersion` becomes the build tag.
4. Run `helm dependency update charts/insight` to regenerate `Chart.lock` from `file://` subcharts.
5. `helm package charts/insight -d dist/`.
6. `helm registry login ghcr.io` using the workflow's `GITHUB_TOKEN`.
7. `helm push dist/insight-<version>.tgz oci://ghcr.io/cyberfabric/charts`.
8. Commit the version bumps back to `main` so the repo state matches what was published.

Resulting OCI artifact: `oci://ghcr.io/cyberfabric/charts/insight:<version>`. The `charts/` segment is part of the GHCR package name (standard Helm-OCI behaviour). The decision rationale lives in [ADR 0001](../specs/ADR/0001-chart-publishing-on-merge.md).

## 3. Step-by-Step Workflow

### 3.1 Code Push to GitHub

An engineer or external contributor merges a pull request into `main` of `cyberfabric/insight`. No special tag, no release, just a merge commit.

GitHub Actions workflow `.github/workflows/build-and-push.yaml` triggers on `push: branches: [main]` and on `workflow_dispatch` for manual reruns.

### 3.2 Image Build and Push to GHCR

The workflow:

1. Checks out the commit.
2. Computes the canonical tag:
   ```bash
   export TAG="$(date -u +%Y.%m.%d.%H.%M)-$(echo "${GITHUB_SHA}" | cut -c1-7)"
   ```
3. Logs in to GHCR with the workflow's `GITHUB_TOKEN` (no PAT).
4. For each service (api-gateway, analytics-api, frontend, identity-resolution, …):
   - `docker buildx build --platform linux/amd64,linux/arm64 -t ghcr.io/cyberfabric/insight-<service>:${TAG} .`
   - `docker push ghcr.io/cyberfabric/insight-<service>:${TAG}`
5. Optionally pushes `:latest` for `main` builds (consumed only by local dev, never by the poller).
6. Emits the tag as a workflow output so the run page shows it without scrolling logs.

The workflow does **not** touch any private system. It does not know GitLab exists. It produces images and stops.

### 3.3 GitLab Poller Updates Manifests

A scheduled GitLab CI job `chart-poller` runs once per hour (cron `0 * * * *`) on a runner inside the corporate network. Its single responsibility: detect new umbrella chart versions on GHCR and bump the gitops pin for environments listed in `auto_envs`. It does **not** deploy.

Logic (pseudocode, implemented in `infra/insight-gitops/scripts/poller.sh`):

```
chart_repo=$(yq '.chart_repository' .poller.yaml)   # oci://ghcr.io/cyberfabric/charts/insight
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
- The poller acts only on environments listed in `auto_envs` of `.poller.yaml`. `dev` is included; non-dev envs (the internal `stage`/`test` clusters and every customer-named production cluster — `virtuozzo`, `constructor`, `acronis`, …) are **not** auto-polled, those bumps are PR'd by an engineer, see [§3.4](#34-engineer-pulls-and-deploys).
- The poller does not write to per-environment values files. Image tags in env values are expected to be empty (the chart's per-subchart `appVersion` flows through). Hotfix-style explicit `image.tag` overrides are an engineer-authored MR, never a poller action.
- A failed `git push` (e.g. someone else pushed a manual change in the same hour) retries with `git pull --rebase` once; on a second failure it leaves the repo dirty and surfaces a CI failure.

The poller does **not** trigger a deploy. It does **not** call ArgoCD. It does **not** notify the cluster. It is a one-way reflector from GHCR to GitLab.

### 3.4 Engineer Pulls and Deploys

A deploy is always initiated by a human at a workstation. Steps:

1. **Sync** — `cd ~/work/insight-gitops && git pull --ff-only origin main`. The pre-flight check in [§6.3](#63-pre-flight-safety-checks) refuses to deploy if `HEAD` is not equal to `origin/main`.
2. **Inspect** — `git log --oneline origin/main ^HEAD@{upstream}` (run by `make diff`) shows the poller commits since the last deploy.
3. **VPN check** — `make deploy ENV=dev` runs the cluster reachability probe before doing any work.
4. **Diff** — the Makefile renders the chart with the current values (`helm template …`) and stores the rendered manifest under `.deploy/last-render-${ENV}.yaml`. The engineer can `diff` against the previous render to see what is changing on the cluster.
5. **Apply** — `helm upgrade --install insight $CHART --version $(cat .insight-version) -n insight -f environments/${ENV}/values.yaml`, where `$CHART = oci://ghcr.io/cyberfabric/charts/insight`. The chart is pulled from GHCR at deploy time; the gitops repo does **not** vendor it — see [§7](#7-repository-layout-target). The Makefile passes `--atomic --timeout 10m` so a failed deploy is rolled back automatically.
6. **Verify** — `make status ENV=dev` runs `kubectl rollout status` for each deployment + `helm test` for smoke tests.

For every non-`dev` environment — both the internal `test` and `stage` clusters and every customer-named production cluster (`virtuozzo`, `constructor`, `acronis`, …; one entry per customer install, no generic "prod"):

- The poller does not auto-bump the chart pin. An engineer opens a merge request that bumps `environments/<env>/.insight-version` (or the umbrella values file) to the desired version (typically the one currently green on `dev`).
- After review and merge, the engineer runs `make deploy ENV=<env>` from their workstation.
- For environments listed in the Makefile's `PROTECTED_ENVS` (every customer cluster; internal `test`/`stage` are at the team's discretion), `make deploy` requires an additional `CONFIRM=yes-deploy-<env>` flag — e.g. `CONFIRM=yes-deploy-virtuozzo` — so a typo on a sleepy morning does not push to a customer cluster. See [§6.2](#62-public-targets) and [§6.3](#63-pre-flight-safety-checks) for the safety check.

## 4. Security Implementation

### 4.1 Secret Lifecycle

There are three distinct states for any piece of secret material:

| State | Where it lives | How to read |
|-------|----------------|-------------|
| Raw secret | Passbolt resource named `insight-<env>-<base>` (password field carries the full cleartext Kubernetes Secret YAML) | `passbolt resource get --name "insight-<env>-<base>" --jsonPassword \| jq -r .password` |
| Sealed manifest | `infra/insight-gitops/environments/<env>/sealed-secrets/<namespace>/<name>-sealedsecret.yaml` (committed) | Anyone with repo read access; opaque to humans |
| In-cluster Secret | Kubernetes API, decrypted by `sealed-secrets-controller` | `kubectl get secret <name> -o yaml` (RBAC-gated) |

The flow between states is one-way at write-time:

```
Passbolt ─(engineer + kubeseal)─▶ Sealed manifest ─(controller)─▶ In-cluster Secret
```

There is no path that puts a raw secret on disk in cleartext between Passbolt and the sealed manifest. The Makefile streams `passbolt resource get` straight into `kubeseal` (see [§4.3](#43-sealed-secrets-sealing-flow)).

### 4.2 Passbolt Integration

- Authoritative store for raw passwords, OIDC client secrets, database passwords, GHCR pull secrets, TLS keys.
- **Storage convention**: one Passbolt resource per Kubernetes Secret per environment. The resource's **password field carries the entire cleartext Kubernetes Secret YAML**, ready to be piped to `kubeseal` without further composition. The resource's URI/username/description fields are documentation only (e.g. `kubectl-namespace=insight`, `kubectl-name=insight-oidc`).
- **Naming**: `insight-<env>-<base>` (e.g. `insight-dev-oidc`, `insight-virtuozzo-db-creds`). The Makefile defaults `PASSBOLT_NAME` to this expression so the engineer rarely passes it explicitly.
- **Authentication**: each engineer's Passbolt account is bound to their personal GPG keypair. `passbolt configure` is run once per workstation to register the server URL, the user, and the private key; subsequent `passbolt resource get` decrypts via the local GPG agent (passphrase cached in the OS keychain). CI never authenticates to Passbolt — the sealing step is a human action.
- The `passbolt` CLI (community: [`go-passbolt-cli`](https://github.com/passbolt/go-passbolt-cli)) is the only sanctioned way to read a secret. Browser-extension copy/paste, screenshots, or pasting into chat are explicitly not.

### 4.3 Sealed Secrets Sealing Flow

`kubeseal` encrypts a Kubernetes `Secret` against the cluster's sealed-secrets-controller public certificate. The encrypted output is committable to Git.

The Makefile target `seal-secret` (see [§6.5](#65-sealed-secret-targets)) implements the streaming flow:

```bash
# Convention: the Passbolt resource named "insight-${ENV}-${NAME}" has,
# in its password field, the complete cleartext Kubernetes Secret YAML
# for ${NAME} on ${ENV}. The pipe below never materialises it on disk.
passbolt resource get --name "insight-${ENV}-${NAME}" --jsonPassword \
  | jq -r .password \
  | kubeseal --format yaml \
      --cert "environments/${ENV}/pub-cert.pem" \
  > "environments/${ENV}/sealed-secrets/${NAMESPACE}/${NAME}-sealedsecret.yaml"
```

Properties:

- The raw secret lives in the pipe only; never on disk, never in shell history. `passbolt resource get | jq -r .password | kubeseal …` is the canonical form.
- The Passbolt resource holds the **whole** Kubernetes Secret manifest (including `apiVersion`, `metadata.name`, `metadata.namespace`, `type`, and every key under `stringData`). `kubeseal` reads it as one object, so no `kubectl create` step is needed. Multi-key secrets (an OIDC client with seven fields) cost no more than single-key secrets.
- `pub-cert.pem` is the cluster controller's public certificate, fetched once per environment and committed to the repo at `environments/<env>/pub-cert.pem`. Renewal procedure is in §8 Open Items.
- The output file is committed. The plaintext input is not, because it never existed as a file.

### 4.4 In-Repo Rules

These are non-negotiable rules enforced by review and by a pre-commit hook in `infra/insight-gitops`:

1. **No plain secrets in Git.** A pre-commit hook runs `gitleaks` against the staged diff and refuses commits that match its rule set.
2. **Sealed manifests only as `*-sealedsecret.yaml`.** Templates (which contain example/empty values) live alongside as `*-secret-template.yaml` and are explicitly listed in the hook's allowlist.
3. **Public certificate is the only key material in Git.** Private keys and Bitnami sealed-secrets-controller's master key live in the cluster only.
4. **No `.env` files.** Local development reads from `passbolt` directly (`passbolt resource get … | jq -r .password`), keeping the cleartext in process memory.

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

- **GitHub** — read access to `cyberfabric/insight` (public) is enough for clones; pushes are protected and only the CI workflow's `GITHUB_TOKEN` can publish images.
- **GitLab** — engineer authenticates with SSH key (`gitlab.cyberfabric.internal`); the deploy-key for the poller is separate and lives only in the GitLab CI variables store.
- **GHCR** — pulls are public; the cluster's image-pull secret is only needed if the team flips an image to private later. The pull secret itself is a sealed secret in the repo.
- **Kubernetes** — engineer's kubeconfig is generated by the corporate IdP; per-cluster contexts follow `insight-<env>` (e.g. `insight-dev`, `insight-stage`, `insight-virtuozzo`, `insight-constructor`). The Makefile checks `kubectl config current-context` against the requested `ENV` before any apply.
- **Passbolt** — `passbolt configure` is run once per workstation: it asks for the server URL, the user's private GPG key file, and the key passphrase. Subsequent `passbolt resource get` invocations decrypt via the local GPG agent; the passphrase is cached in the OS keychain for the agent's TTL.

### 5.3 VPN and Cluster Access

- All Kubernetes API endpoints resolve only when the corporate VPN is up. The Makefile's pre-flight runs `kubectl --request-timeout=5s cluster-info` and aborts with "VPN not connected?" if the call fails.
- DNS for `*.cyberfabric.internal` is split-horizon: workstations on VPN see internal addresses; off-VPN they see nothing. The poller's GitLab runner is permanently in-network.
- There is **no** ingress from public networks to either GitLab or the clusters. A compromise of GHCR yields only the ability to publish a malformed image, which would still need to be picked up by the poller and approved by an engineer for any non-`dev` env.

## 6. Makefile Specifications

### 6.1 Variables and Defaults

```make
# infra/insight-gitops/Makefile

ENV              ?= dev
NAMESPACE        ?= insight
RELEASE          ?= insight
CHART            ?= oci://ghcr.io/cyberfabric/charts/insight
INSIGHT_VERSION  ?= $(shell cat .insight-version)
VALUES           ?= environments/$(ENV)/values.yaml
KUBE_CTX         ?= insight-$(ENV)
TIMEOUT          ?= 10m
RENDER_DIR       := .deploy
```

- All variables are overridable on the command line (`make deploy ENV=stage`).
- `ENV` controls which values file, which kube-context, which sealed-secrets directory.
- `INSIGHT_VERSION` defaults to the contents of `.insight-version` at the repo root — the umbrella semver currently pinned for this repo. Override only for ad-hoc one-off renders (`make diff INSIGHT_VERSION=0.1.42`).
- The `.deploy/` directory is `.gitignore`d and stores the last rendered manifest plus a per-deploy log file.

### 6.2 Public Targets

| Target | Purpose | Pre-flight | Effect |
|--------|---------|------------|--------|
| `make doctor` | Verify required tooling and auth. | none | Read-only checks; prints status. |
| `make sync` | `git fetch && git pull --ff-only origin main`. | none | Updates local repo to match `origin/main`. |
| `make diff` | Show poller commits since last deploy and rendered-manifest diff. | `sync-clean` | Read-only. Prints commit list and `helm template` diff. |
| `make deploy` | Apply the chart to the cluster. | `sync-clean`, `vpn-up`, `kube-ctx`, `confirm` (only fires for envs in `PROTECTED_ENVS`) | `helm upgrade --install --atomic`. |
| `make rollback` | Roll back to the previous Helm revision. | `vpn-up`, `kube-ctx` | `helm rollback`. |
| `make status` | Show release status and rollout health. | `vpn-up`, `kube-ctx` | Read-only. |
| `make tag` | Tag the deploy commit as `deploy-…`. | `sync-clean` | Local + remote git tag. Optional. |
| `make seal-secret NAME=… NAMESPACE=… [PASSBOLT_NAME=…]` | Seal the cleartext Secret YAML stored in Passbolt into a sealed-secret manifest. `PASSBOLT_NAME` defaults to `insight-$(ENV)-$(NAME)`. | `passbolt-configured` | Writes one `*-sealedsecret.yaml`. |
| `make clear-seal-template NAME=…` | Reset a template file to empty values. | none | Edits the template in place. |

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
# customers come online — virtuozzo, constructor, acronis, …
PROTECTED_ENVS := virtuozzo

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
- `confirm` is a deliberately ugly flag, scoped per env. If you can type `CONFIRM=yes-deploy-virtuozzo`, you have looked at it. Each customer cluster requires its own token (`yes-deploy-constructor`, `yes-deploy-acronis`, …) so muscle memory does not carry across customers.
- `passbolt-configured` is checked at the start of `seal-secret` rather than inside the pipe so the failure message is clear.

### 6.4 Deploy Logic

```make
.PHONY: deploy-insight
deploy-insight: sync-clean vpn-up kube-ctx confirm chart-present
	@mkdir -p $(RENDER_DIR)
	@helm template $(RELEASE) $(CHART) --version $(INSIGHT_VERSION) \
		-n $(NAMESPACE) -f $(VALUES) \
		> $(RENDER_DIR)/last-render-$(ENV).yaml
	@echo "Rendered $(CHART):$(INSIGHT_VERSION) to $(RENDER_DIR)/last-render-$(ENV).yaml"
	helm upgrade --install $(RELEASE) $(CHART) \
		--version $(INSIGHT_VERSION) \
		--namespace $(NAMESPACE) --create-namespace \
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

- `$(CHART)` resolves to `oci://ghcr.io/cyberfabric/charts/insight`; `$(INSIGHT_VERSION)` resolves to the contents of `.insight-version`. Together they uniquely identify the published artifact.
- `chart-present` fails fast if the OCI tag does not exist — saves the engineer a confusing `helm upgrade` error.
- `helm template` first lets the engineer abort if the rendered diff looks wrong (a follow-up `make plan` target compares against `kubectl get` to surface the actual cluster diff; out of scope for v0).
- `--atomic` rolls back on failure; combined with `--timeout 10m` it bounds blast radius.
- `--history-max 10` keeps Helm's internal release history bounded so `make rollback` always has a target.
- The top-level `deploy` target orchestrates the per-cluster prerequisites (Airbyte, Argo Workflows) and then calls `deploy-insight`. See the gitops repo's Makefile for the orchestration; only the `deploy-insight` step interacts with the OCI-pinned umbrella chart.

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

The implementation phase materialises this layout in `infra/insight-gitops`:

```
infra/insight-gitops/
├── Brewfile
├── Makefile
├── README.md
├── .insight-version            # one line: the umbrella semver pin (e.g. 0.1.47)
├── .poller.yaml                # chart_repository + version_pin_file + auto_envs
├── .gitlab-ci.yml              # defines the chart-poller scheduled job
├── .gitleaks.toml              # secret-scanning rules
├── base-values/
│   ├── airbyte-values.yaml     # vendored from cyberfabric/insight at a known SHA
│   └── argo-values.yaml
├── bootstrap/
│   └── argo-rbac.yaml.tmpl     # templated; substituted at apply time
├── environments/
│   ├── dev/                       # internal — auto-bumped by the poller
│   │   ├── values.yaml
│   │   ├── airbyte-values.yaml
│   │   ├── argo-values.yaml
│   │   ├── pub-cert.pem
│   │   └── sealed-secrets/
│   │       └── insight/
│   │           ├── oidc-client-sealedsecret.yaml
│   │           └── db-creds-sealedsecret.yaml
│   ├── stage/                     # internal — promote-by-MR (optional)
│   │   └── …                      # same shape as dev/
│   ├── virtuozzo/                 # customer prod — promote-by-MR + confirm token
│   │   ├── values.yaml
│   │   ├── airbyte-values.yaml
│   │   ├── argo-values.yaml
│   │   ├── pub-cert.pem
│   │   └── sealed-secrets/...
│   └── <other-customer>/          # one dir per customer install (constructor, acronis, …)
│       └── …                      # same shape
└── scripts/
    ├── poller.sh               # invoked by .gitlab-ci.yml on cron
    ├── doctor.sh               # invoked by `make doctor`
    ├── render-diff.sh          # invoked by `make diff`
    └── airbyte-setup.sh        # post-install Airbyte setup-wizard automation
```

Conventions:

- One `values.yaml` per environment for the umbrella chart, plus one `airbyte-values.yaml` and one `argo-values.yaml` per environment as overlays on top of `base-values/`. The umbrella chart already exposes one flat values surface; per-engine files exist only because Airbyte and Argo Workflows are separate Helm releases.
- One `pub-cert.pem` per environment because each cluster runs its own sealed-secrets-controller with its own keypair.
- **The umbrella chart lives in `cyberfabric/insight` and is published per merge to `oci://ghcr.io/cyberfabric/charts/insight`.** The gitops repo is settings-only — values, sealed secrets, the Makefile, the poller. It does **not** vendor the chart and does **not** require a local checkout of `cyberfabric/insight`. The Makefile pulls the chart from OCI at deploy time, pinned to the version in `.insight-version`.

## 8. Open Items

These are accepted gaps that do not block the MVP but must be tracked.

- **Public certificate rotation.** The sealed-secrets-controller rotates its keypair periodically; when it does, the committed `pub-cert.pem` files go stale and previously sealed secrets continue to decrypt (old keys are kept), but new ones must be sealed against the new cert. Procedure: `kubeseal --fetch-cert > environments/<env>/pub-cert.pem`, commit, re-seal any in-flight changes. A scheduled monthly check is appropriate; not yet automated.
- **Promotion-MR poller for non-`dev` envs.** Currently only `dev` is auto-bumped. For internal `stage`/`test` and every customer-named cluster (`virtuozzo`, `constructor`, `acronis`, …), the team may want a "dry-run" poller that opens a merge request rather than committing to `main`. Captured but not yet designed.
- **Migration to in-cluster ArgoCD.** The Makefile-driven manual deploy is an MVP shortcut. Once a managed ArgoCD instance is provisioned inside the corporate network, the same `infra/insight-gitops` repo becomes its source. The contract (one `values.yaml` per environment, sealed secrets per namespace) is designed to survive that migration unchanged; only the trigger mechanism changes from `make deploy` to ArgoCD reconciliation.
- **Artifact signing (images + chart).** Neither GHCR images nor the umbrella Helm chart at `oci://ghcr.io/cyberfabric/charts/insight` are signed today. The deploy admits any image tag the poller resolves and any chart version `.insight-version` pins. Follow-up: cosign-sign both at publish time, have `make chart-present` verify the chart signature before allowing deploy, and add `cosign verify` to the cluster admission policy for images.
- **Audit log of deploys.** `make deploy` writes a local log file; there is no central audit. A trivial follow-up posts the log to a `#deploys` Slack channel via the poller's bot token; deferred until the team needs it.
- **Rollback-by-tag.** `make rollback` calls `helm rollback` to the previous revision. Rolling back to an arbitrary historical state is `git checkout <deploy-tag> && make deploy`, which works but has not been rehearsed.
