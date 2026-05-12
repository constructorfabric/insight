---
status: proposed
date: 2026-05-12
---

# PRD — Deployment

## Table of Contents

1. [1. Overview](#1-overview)
   - [1.1 Purpose](#11-purpose)
   - [1.2 Background / Problem Statement](#12-background--problem-statement)
   - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
   - [1.4 Glossary](#14-glossary)
2. [2. Actors](#2-actors)
   - [2.1 Human Actors](#21-human-actors)
   - [2.2 System Actors](#22-system-actors)
3. [3. Operational Concept & Environment](#3-operational-concept--environment)
   - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
4. [4. Scope](#4-scope)
   - [4.1 In Scope](#41-in-scope)
   - [4.2 Out of Scope](#42-out-of-scope)
5. [5. Functional Requirements](#5-functional-requirements)
   - [5.1 Umbrella Chart Packaging](#51-umbrella-chart-packaging)
   - [5.2 Constructor Platform Integration](#52-constructor-platform-integration)
   - [5.3 Chart Publishing and Distribution](#53-chart-publishing-and-distribution)
   - [5.4 Layered Deploy Model and Customer Envs](#54-layered-deploy-model-and-customer-envs)
   - [5.5 Developer Workflow](#55-developer-workflow)
   - [5.6 Multi-Tenant Deployment](#56-multi-tenant-deployment)
   - [5.7 Credential Hygiene](#57-credential-hygiene)
6. [6. Non-Functional Requirements](#6-non-functional-requirements)
   - [6.1 NFR Inclusions](#61-nfr-inclusions)
   - [6.2 NFR Exclusions](#62-nfr-exclusions)
7. [7. Public Library Interfaces](#7-public-library-interfaces)
   - [7.1 Public API Surface](#71-public-api-surface)
   - [7.2 External Integration Contracts](#72-external-integration-contracts)
8. [8. Use Cases](#8-use-cases)
   - [8.1 Eval install on a laptop](#81-eval-install-on-a-laptop)
   - [8.2 Constructor Platform tenant install](#82-constructor-platform-tenant-install)
   - [8.3 Developer inner loop](#83-developer-inner-loop)
9. [9. Acceptance Criteria](#9-acceptance-criteria)
10. [10. Dependencies](#10-dependencies)
11. [11. Assumptions](#11-assumptions)
12. [12. Risks](#12-risks)

## 1. Overview

### 1.1 Purpose

The Deployment subsystem produces **one releasable artifact** for the Insight platform — the `insight` umbrella Helm chart, published per-merge to `oci://ghcr.io/cyberfabric/charts/insight:<semver>`. That single artifact is consumed by two distinct paths: the private `infra/insight-gitops` repository, which drives every Cyberfabric-operated cluster (internal `dev` / `test` / `stage` and each customer-named production cluster — `virtuozzo`, `constructor`, `acronis`, …), and any external Helm-aware consumer that wants to run Insight on their own terms (helm, ArgoCD, Flux, kustomize render). The same chart also powers the developer bring-up wrapper `dev-up.sh`, which targets a local Kind/OrbStack cluster.

The subsystem does not ship product functionality on its own — it composes the application services (API Gateway, Analytics API, Frontend, optional Identity Resolution) with their required infrastructure (ClickHouse, MariaDB, Redis, Redpanda, Airbyte, Argo Workflows) into a versioned chart, enforces the contracts between them (single-namespace dev mode, layered L0/L2/L3 gitops mode, external-mode infra contracts, fail-fast validation, mandatory OIDC in production) and provides the dev wrapper. Orchestration of *customer* installs that are not Cyberfabric-operated is explicitly out of scope: external chart consumers pick their own tooling.

### 1.2 Background / Problem Statement

Before the consolidation captured in [ADR-0001](./ADR/0001-chart-publishing-on-merge.md), the Insight stack was distributed through three overlapping paths: a canonical shell installer (`deploy/scripts/install.sh`), an ArgoCD App-of-Apps shipped in the public repo (`deploy/gitops/`), and the developer wrapper. Each path had its own assumptions about where the chart came from, how images were tagged, and what the operator had to clone. The result was structural drift — engineers overriding `image.tag` per service in environment values while a chart-shape change shipped on an independent cadence — and three documentation surfaces for what should have been one product.

Two concrete pain points drove the rewrite. First, Cyberfabric SRE needs to ship the same chart to a growing roster of customer-named production clusters from a private settings-only repo, without forcing every operator workstation to clone the public application repo or to hand-pick image tags. Second, external chart consumers (Constructor Platform, enterprise evaluators) need a stable artifact reference they can pin in their own tooling — not a recipe for assembling an install from a sibling checkout. Both demand a single versioned artifact published per merge with the chart shape and the images coming from the same CI run on the same commit.

The third driver is reproducibility for the development team itself: a developer joining the project should be able to clone the repo, run one script, and end up with a live stack that mirrors the production topology — so that layout bugs are caught in dev rather than in a customer environment. The dev wrapper therefore reuses the same chart, in single-namespace mode via per-service `<svc>.deploy: true` toggles, instead of a parallel dev-only chart.

### 1.3 Goals (Business Outcomes)

- Eliminate chart-vs-image drift structurally: every published umbrella version bundles a known set of image tags (each subchart's `appVersion`), so pinning the umbrella semver in a gitops repo pins both chart shape and images atomically. Measured by absence of out-of-band `image.tag` overrides in env values files after the migration.
- Reduce the time from "Cyberfabric SRE merges a fix" to "the fix is running on `dev`" to under one hour, measured end-to-end from PR merge to pod rollout, by automating chart publish + poller-driven `.insight-version` bump.
- Enable Constructor Platform onboarding by allowing each infra dependency to be flipped from bundled to external via a single `<dep>.deploy: false` toggle plus the same flat `host` / `port` / `passwordSecret` fields the bundled mode reads, so a shared-platform tenant install reuses the platform's ClickHouse / MariaDB / Redpanda without code changes.
- Keep developer inner-loop under 10 minutes from `dev-up.sh` to a usable cluster with locally built images, so platform changes can be tested against a realistic topology before review.
- Prevent accidental shipping of default passwords or placeholder secrets by failing `helm install` fast when credentials are empty and no external Secret is declared.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Umbrella chart | The `charts/insight/` Helm chart that aggregates all Insight subcharts (infra + app services + ingestion templates) via Chart.yaml dependencies. Published per merge to `oci://ghcr.io/cyberfabric/charts/insight:<semver>`. |
| Chart Publishing CI | The GitHub Actions workflow in `cyberfabric/insight` that, on every merge to `main`, builds the changed service images, bumps the affected subcharts' `appVersion` to the build tag, patch-bumps the umbrella's `version`, sets the umbrella `appVersion` to the build tag, packages the chart, and pushes it to GHCR. |
| Dev wrapper | `dev-up.sh` (and `dev-down.sh`) — bring-up scripts that build images from source, create a local Kind/OrbStack cluster, and install the same umbrella chart with all `<svc>.deploy: true` for a single-namespace fat install. |
| Gitops repo | Private `infra/insight-gitops` settings-only repository on internal GitLab that drives every Cyberfabric-operated cluster. Pins exactly one umbrella semver per environment via `.insight-version` and pulls the chart from OCI at deploy time; does **not** vendor the chart. |
| L0 / L2 / L3 | Three deploy layers used by gitops production: **L0 Bootstrap** (cluster prerequisites — sealed-secrets-controller, ingress-nginx, cert-manager — cluster-scoped, runs once per cluster); **L2 System** (shared stateful infra — MariaDB, ClickHouse, Redis, Redpanda + Console, Airbyte, Argo Workflows — one Helm release per service in the `insight-infra` namespace, each replaceable by a managed external endpoint); **L3 App** (the umbrella chart, app services only, in the `insight` namespace). There is no L1 — that number is reserved for cluster provisioning, which is out of scope. |
| Customer-named env | A production environment named after the customer that owns it (`virtuozzo`, `constructor`, `acronis`, …). The gitops repo has no generic "prod"; each customer install is its own env directory and its own `kubectl` context (`insight-<env>`). |
| Dual-purpose umbrella | One chart, two install shapes selected by `<svc>.deploy` toggles. `dev-up.sh` flips them all `true` (single fat `insight` namespace with bundled infra); gitops production flips them all `false` (app services only in `insight`, infra elsewhere). |
| External mode | State of an infra dependency where `<dep>.deploy: false`. The umbrella does not run the bundled subchart; consumers read the same flat `<dep>.host`, `<dep>.port` and `<dep>.passwordSecret` fields and the Secret is provided by the operator (or platform). The same shape used by gitops production cross-namespace wiring (`<svc>.insight-infra.svc.cluster.local`) and by Constructor Platform tenant installs (platform-issued endpoints). |
| Constructor Platform | Shared multi-product infrastructure fabric operated by the vendor. It provides ClickHouse, MariaDB, Redpanda and identity services that tenant products consume via external-mode contracts. |
| Platform ConfigMap | The single `{release}-platform` ConfigMap emitted by the umbrella that contains resolved infra coordinates (CLICKHOUSE_URL, MARIADB_HOST, AIRBYTE_API_URL, …). Pods consume it via `envFrom`. |
| `.insight-version` | One-line file at the root of `infra/insight-gitops` containing the umbrella semver currently pinned for the repo. The poller bumps it on auto-bumped envs; engineers bump it via merge request for customer-named envs. |
| Eval credentials | Throwaway passwords in `deploy/values-dev.yaml` used only by dev bring-up and short-lived eval clusters; never shipped to production. |

## 2. Actors

### 2.1 Human Actors

#### Customer SRE / External Chart Consumer

**ID**: `cpt-insightspec-actor-customer-sre`

**Role**: External operator (customer SRE, partner, evaluator) who pulls the published umbrella chart from `oci://ghcr.io/cyberfabric/charts/insight` and installs it on a Kubernetes cluster they own, using whatever tooling they prefer (helm, ArgoCD, Flux, Terraform Helm provider, Argo Rollouts, custom GitOps). They are **not** Cyberfabric-operated and are not granted access to `infra/insight-gitops`.
**Needs**: A stable artifact reference with semver tags, a documented values contract (the chart README), explicit documentation for external-mode overrides, a path to roll back failed upgrades, and clear failure messages when required values are missing. Does not need an opinionated installer — picks their own orchestration.

#### Constructor Platform Operator

**ID**: `cpt-insightspec-actor-platform-operator`

**Role**: Internal operator who onboards Insight as a tenant of the Constructor Platform, wiring it to the shared ClickHouse / MariaDB / Redpanda. Consumes the published chart from the same OCI artifact reference as external SREs.
**Needs**: Per-infra `deploy` flags plus a single flat block (`host`, `port`, `database`, `username`, `passwordSecret`) that the chart reads identically whether the dependency is bundled or external, and a validator that fails fast when any of those are missing.

#### Cyberfabric SRE

**ID**: `cpt-insightspec-actor-cyberfabric-sre`

**Role**: Internal operator running deploys against Cyberfabric-operated clusters (`dev`, `test`, `stage`, and every customer-named production cluster — `virtuozzo`, `constructor`, `acronis`, …). Works from the private `infra/insight-gitops` settings repo through its `Makefile`, on a workstation with VPN + kubeconfig.
**Needs**: One Makefile-driven workflow that covers L0 bootstrap, L2 system service installs and L3 umbrella deploys against any env; a one-file promotion mechanism (`.insight-version` bump); per-customer deploy gating (`PROTECTED_ENVS` + `CONFIRM=yes-deploy-<env>` token) so muscle memory does not push the wrong values to the wrong cluster; sealed-secrets workflow fed from Passbolt.

#### Platform Developer

**ID**: `cpt-insightspec-actor-platform-developer`

**Role**: Engineer on the Insight team iterating on the services, charts or ingestion code.
**Needs**: A single dev wrapper that builds images locally, bootstraps a Kind cluster, applies dev overlays, port-forwards the relevant services, and exercises the same umbrella chart that customers run (with `<svc>.deploy: true` flipped on for in-cluster bundled infra) so bugs show up before release.

### 2.2 System Actors

#### Kubernetes Cluster

**ID**: `cpt-insightspec-actor-kubernetes`

**Role**: Target runtime. The Deployment subsystem targets Kubernetes 1.27+ (declared in the umbrella's `Chart.yaml` via `kubeVersion`), served either by Kind/OrbStack locally, by an internal Cyberfabric cluster (`insight-dev`, internal `stage`/`test`, customer-named production clusters), or by an external customer-owned cluster consuming the chart.

#### Helm

**ID**: `cpt-insightspec-actor-helm`

**Role**: Package manager used by every consumer path (the gitops Makefile, `dev-up.sh`, and any external consumer). The umbrella ships as a Helm chart at `oci://ghcr.io/cyberfabric/charts/insight`; Airbyte and Argo Workflows are their upstream Helm charts pinned per L2 system release (gitops) or installed via dev-helper scripts (`dev-up.sh`).

#### Chart Publishing CI

**ID**: `cpt-insightspec-actor-chart-publishing-ci`

**Role**: GitHub Actions workflow in `cyberfabric/insight` that, on every merge to `main`, builds the changed service images, bumps the affected subcharts' `appVersion` to the build tag, patch-bumps the umbrella's `version`, sets the umbrella `appVersion` to the build tag, runs `helm dependency update`, packages the umbrella and pushes it to `oci://ghcr.io/cyberfabric/charts/insight:<semver>`. Auto-commits the version bumps back to `main` so the repo state matches what was published.

#### Argo Workflows Controller

**ID**: `cpt-insightspec-actor-argo-workflows`

**Role**: Engine that executes the ingestion `WorkflowTemplates` emitted by the umbrella. Installed as a separate Helm release — in `insight-infra` for gitops production (L2), in the `insight` namespace for `dev-up.sh` local — scoped to the install via `controller.instanceID` and `controller.workflowNamespaces`.

#### Airbyte Engine

**ID**: `cpt-insightspec-actor-airbyte-engine`

**Role**: Data extraction engine. Installed as a separate Helm release in `insight-infra` (gitops production) or the `insight` namespace (`dev-up.sh` local), pinned to chart 1.8.5+ / app 1.8.5+ per release. Post-install setup-wizard automation completes Airbyte's one-time setup via its REST API so the UI is usable on first visit.

#### OCI Artifact Registry (GHCR)

**ID**: `cpt-insightspec-actor-artifact-registry`

**Role**: Source of record for the umbrella chart (`oci://ghcr.io/cyberfabric/charts/insight`) and application images (`ghcr.io/cyberfabric/insight-*`). Public, read-only to consumers; written by Chart Publishing CI from `cyberfabric/insight` on merge to `main`. The chart `charts/` URL segment is part of the GHCR package name (standard Helm-OCI behaviour).

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Target Kubernetes version: 1.27 or newer (declared via `kubeVersion: ">=1.27.0-0"` in the umbrella `Chart.yaml`).
- Helm 3.14+ required for OCI chart pulls and registry authentication.
- Docker Desktop / Rancher Desktop / OrbStack / Podman with working containerd image load required on the developer path (the dev wrapper uses `kind load docker-image`).
- Bitnami chart dependencies (MariaDB, Redis) are pinned to the `bitnamilegacy` registry variants with `global.security.allowInsecureImages: true`, because Bitnami moved free images off `docker.io/bitnami/*` in 2025.
- Frontend image is currently published as `linux/amd64` only. On Apple Silicon hosts the dev wrapper rebuilds the frontend from the sibling `insight-front` checkout rather than pulling the upstream image; production installs rely on Docker Desktop's QEMU emulation or a customer-side multi-arch mirror.
- The umbrella chart assumes release name `insight` for its internal DNS references inside `values.yaml`. Using a non-default release name requires overriding the affected URL fields.
- For gitops production, each cluster carries exactly one Insight install; the cluster's identity (which env it represents) lives in the kube-context name (`insight-<env>`) and the gitops repo's `environments/<env>/` directory — not in the namespace. The two well-known namespaces (`insight-infra` for L2, `insight` for L3) are the same across every install.
- The dev wrapper targets Kind 0.22+ / OrbStack; cluster bootstrapping uses a fixed cluster name `insight` to match hard-coded port mappings.

## 4. Scope

### 4.1 In Scope

- The Insight umbrella Helm chart at `charts/insight/` with eight declared dependencies (ClickHouse, MariaDB, Redis, Redpanda, API Gateway, Analytics API, Frontend, Identity Resolution).
- The service-resolution helper library (`templates/_helpers.tpl`) that returns the same values whether a dependency is bundled or external, and the `insight.validate` template that fails rendering on missing required fields.
- The single `{release}-platform` ConfigMap that exposes resolved infra coordinates to every pod in the namespace via `envFrom`.
- Argo `WorkflowTemplate` emission as first-class Helm templates under `charts/insight/templates/ingestion/*.yaml`, gated by `ingestion.templates.enabled` and consuming umbrella helpers (`insight.clickhouse.fqdn`, `insight.airbyte.url`, …) directly via `include`.
- The dual-purpose `<svc>.deploy: true|false` toggle: same chart powers `dev-up.sh` single-namespace fat installs and gitops production layered installs.
- The Chart Publishing CI workflow that publishes the umbrella to `oci://ghcr.io/cyberfabric/charts/insight:<semver>` per merge to `main`; the per-subchart `appVersion = image tag` contract; the umbrella semver versioning rules; `.insight-version` as the single gitops pin.
- Developer bring-up wrappers `dev-up.sh` / `dev-down.sh` / `init.sh`, parameterised by `INSIGHT_NAMESPACE`, with Kind bootstrap, image build + `kind load`, and port-forwards for the common UIs.
- Dev-only credential overlay `deploy/values-dev.yaml` (throwaway passwords) applied automatically by `dev-up.sh`.
- The DEVLOG.md that records the first-run debugging narrative so future developers and dev-up users can resolve the same twelve issues without rediscovering them.
- The chart README (`charts/insight/README.md`) as the values contract for every consumer.

### 4.2 Out of Scope

- **Orchestration of customer / external installs.** Anyone consuming the chart from OCI picks their own tooling (helm, ArgoCD, Flux, Terraform Helm provider, kustomize render, custom GitOps). The Deployment subsystem does not ship a customer-facing installer script; it ships the chart and documents the values contract.
- **The `infra/insight-gitops` repository content itself.** That repo is owned operationally by Cyberfabric SRE and lives on private GitLab. Its design and operational contract are captured in the [gitops SPEC](../gitops/README.md); the public deployment PRD/DESIGN documents the chart-as-artifact and the dev wrapper, not the gitops repo's internals.
- Multi-architecture (linux/arm64) frontend image publication.
- Bidirectional sync between the umbrella-managed `insight-db-creds` Secret and a customer-supplied secret-management system (Vault, AWS Secrets Manager, External Secrets Operator). Customers integrating with such systems pre-create `insight-db-creds` themselves (the chart auto-detects BYO via absence of the `app.kubernetes.io/managed-by=Helm` label and skips its own Secret-template emission); or they accept the auto-generated values and mirror them outwards by their own means.
- Cluster provisioning (creating the customer's Kubernetes cluster, setting up a StorageClass, installing ingress-nginx on a production cluster). The dev wrapper bootstraps Kind for local work; gitops production handles L0 in `infra/insight-gitops` (out of this repo's scope); external chart consumers bring their own cluster.
- Backup, restore, and disaster-recovery workflows for the bundled stateful services (ClickHouse, MariaDB). Mentioned in the Backend PRD; not owned by Deployment.
- Identity Provider (OIDC) provisioning. The deployment contract requires OIDC credentials as input; standing up an IdP is the consumer's responsibility.
- Customer-facing documentation portal. Internal README files, the chart README, the gitops SPEC and DEVLOG.md are in scope; hosted docs are not.

## 5. Functional Requirements

### 5.1 Umbrella Chart Packaging

#### Single umbrella distributable

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-umbrella-chart`

The system **MUST** ship a single Helm umbrella chart named `insight` that aggregates the four infrastructure subcharts (ClickHouse, MariaDB, Redis, Redpanda) and the four application subcharts (API Gateway, Analytics API, Frontend, Identity Resolution) as declared dependencies in `Chart.yaml`, so that a single `helm install insight oci://ghcr.io/cyberfabric/charts/insight --version <X.Y.Z>` renders every Kubernetes object that the platform requires.

**Rationale**: A single artifact is what every consumer (Cyberfabric SRE pinning one version per env, Constructor Platform integrating as a tenant, external evaluators) can version, roll back and audit. Multiple independent releases are not a product.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-cyberfabric-sre`, `cpt-insightspec-actor-platform-operator`

#### Mandatory application services

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-mandatory-apps`

The umbrella chart **MUST** treat API Gateway, Analytics API and Frontend as mandatory dependencies with no per-chart `enabled` flag, because the gateway is the single entrance to the cluster internals and the other services are reachable only through it.

**Rationale**: Hiding any of these behind a boolean creates configurations that install successfully but produce a non-functional product and have historically been shipped by accident.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-platform-operator`

#### Optional Identity Resolution subchart

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-optional-identity-resolution`

The umbrella chart **MUST** treat the `insight-identity-resolution` subchart as optional with `condition: identityResolution.deploy` defaulting to `false`, because that service requires populated bronze data and crash-loops on an empty database.

**Rationale**: A first install has no bronze data; shipping identity-resolution enabled by default would make every first install look broken.

**Actors**: `cpt-insightspec-actor-customer-sre`

#### Argo WorkflowTemplate emission

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-ingestion-templates`

The umbrella chart **MUST** emit the Argo `WorkflowTemplate` objects under `charts/insight/templates/ingestion/` as first-class Helm templates that consume the umbrella's named helpers (`insight.clickhouse.fqdn`, `insight.airbyte.url`, etc.) directly. Argo's own `{{inputs.parameters.*}}` expressions **MUST** be escaped with backtick raw-string literals so they pass through Helm rendering unmodified. Emission is gated by `ingestion.templates.enabled`.

**Rationale**: First-class Helm templating gives `helm lint` coverage, removes a custom placeholder-substitution bridge, and lets pipeline authors call any umbrella helper without round-tripping through values keys. The earlier placeholder-substitution approach was rejected on review for being fragile and uncheckable.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-platform-developer`

#### Platform ConfigMap surface

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-platform-configmap`

The umbrella chart **MUST** render a single ConfigMap named `{release}-platform` containing all resolved infra coordinates (ClickHouse URL, MariaDB host/port/db, Redis host/port/URL, Redpanda brokers, Airbyte API URL, application service hostnames) so that any pod in the release namespace can consume these values via `envFrom` without duplicating DNS names in its own values.

**Rationale**: Centralising resolved coordinates is the long-term path for app services to stop carrying hard-coded URLs in their own `values.yaml`.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-platform-developer`

### 5.2 Constructor Platform Integration

#### External-mode switch per infra dependency

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-external-mode`

Each infrastructure dependency in the umbrella (ClickHouse, MariaDB, Redis, Redpanda) **MUST** expose a single unified shape — `<dep>.deploy: true/false` plus flat `host` / `port` / (where applicable) `database` / `username` / `passwordSecret.{name,key}` — read identically by consumers whether the dependency is bundled (umbrella runs the subchart) or external (umbrella does not run the subchart and the operator points the same fields at a platform-provided instance or a cross-namespace L2 service).

**Rationale**: Constructor Platform tenant installs must reuse the platform's shared ClickHouse / MariaDB / Redpanda; the gitops production model points the same fields at L2 services in `insight-infra`. The umbrella cannot assume every install bundles its own infra.

**Actors**: `cpt-insightspec-actor-platform-operator`, `cpt-insightspec-actor-cyberfabric-sre`

#### Fail-fast validation of external contracts

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-fail-fast-validation`

The umbrella chart **MUST** invoke an `insight.validate` template during rendering that fails rendering with a readable message whenever `<dep>.deploy: false` is used without `<dep>.host`, whenever any `<dep>.passwordSecret.name` or `.key` is missing, whenever a pre-existing `insight-db-creds` Secret is present but a required key is missing or empty (BYO mode), or whenever `apiGateway.authDisabled: false` is set with neither `apiGateway.oidc.existingSecret` nor all three of `issuer` + `clientId` + `redirectUri` populated together.

**Rationale**: Silent defaults or partial configuration produces clusters that install cleanly but fail at runtime — by which time the operator has already lost access to the diagnostic output.

**Actors**: `cpt-insightspec-actor-platform-operator`, `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-cyberfabric-sre`

#### Helper-based service resolution

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-service-resolution-helpers`

The umbrella chart **MUST** resolve every infra host, port, FQDN and URL through named helpers in `_helpers.tpl` (rather than template-time string concatenation) that return the internal cluster-DNS name when a dependency is bundled and the externally-provided host verbatim when it is external, without appending the cluster-DNS suffix to a hostname that already contains a dot.

**Rationale**: Prevents `clickhouse.example.com.insight.svc.cluster.local` mangling in external mode and keeps rename refactors to a single file.

**Actors**: `cpt-insightspec-actor-platform-developer`

### 5.3 Chart Publishing and Distribution

#### Per-merge umbrella chart publish to OCI

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-chart-publishing`

The system **MUST**, on every merge to `main` of `cyberfabric/insight`, build the changed service images, bump the affected subcharts' `appVersion` to the build tag, patch-bump the umbrella's `version`, set the umbrella `appVersion` to the build tag, run `helm dependency update`, package the umbrella chart and push it to `oci://ghcr.io/cyberfabric/charts/insight:<umbrella-version>`. The workflow **MUST** auto-commit the version bumps back to `main` so the repo state matches what was published. The rationale is captured in [ADR-0001](./ADR/0001-chart-publishing-on-merge.md).

**Rationale**: Eliminates chart-vs-image drift structurally — both come from the same CI run on the same commit. One pin per gitops env. Per-service granularity preserved because each subchart's `appVersion` advances independently. Chart consumers outside Cyberfabric get a stable artifact reference.

**Actors**: `cpt-insightspec-actor-chart-publishing-ci`, `cpt-insightspec-actor-artifact-registry`

#### OCI distribution as the single consumer-facing artifact reference

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-oci-distribution`

The umbrella chart **MUST** be addressable by every consumer at `oci://ghcr.io/cyberfabric/charts/insight:<semver>`. Any consumer — Cyberfabric SRE through the gitops Makefile, Constructor Platform, external customers, ArgoCD/Flux users — pulls the chart by that reference and a semver tag. No other public deploy path (no canonical installer, no App-of-Apps shipped from `cyberfabric/insight`) is documented or supported.

**Rationale**: A single artifact reference makes promotion (`.insight-version` bump) atomic and is the only contract external consumers need.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-platform-operator`, `cpt-insightspec-actor-cyberfabric-sre`

#### Per-subchart appVersion equals image tag

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-subchart-appversion-contract`

Each subchart's `values.yaml` **MUST** default `image.tag = ""`, and the templates **MUST** resolve via `default .Chart.AppVersion`. Inside a subchart, `.Chart.AppVersion` resolves to that subchart's own `appVersion` — not the umbrella's. The Chart Publishing CI **MUST** bump each rebuilt subchart's `appVersion` to the build tag of its image so the chart's published shape carries each service's actual image tag.

**Rationale**: Per-service tag granularity is structural, not by convention — rebuilding only `api-gateway` advances only that subchart's `appVersion`, others stay on their prior tags. Env values files do not need `image.tag` overrides in routine use.

**Actors**: `cpt-insightspec-actor-platform-developer`, `cpt-insightspec-actor-chart-publishing-ci`

#### Umbrella semver and appVersion semantics

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-umbrella-versioning`

The umbrella's `Chart.yaml` `version` **MUST** follow semver with patch-bump per CI publish and minor-bump per explicit PR change to the umbrella's own templates or values shape. The umbrella's `appVersion` **MUST** be set to the build tag of the publishing CI run (display only — no template reads it). The gitops repo **MUST** pin exactly one umbrella semver per environment in a one-line file at the root (`.insight-version`); the contract makes promotion a one-line change.

**Rationale**: Semver per publish gives ordered consumable versions; `appVersion` as build tag makes `helm list` and `kubectl describe pod` immediately point back to a Git revision; one pin per env is the smallest atomic promotion unit.

**Actors**: `cpt-insightspec-actor-chart-publishing-ci`, `cpt-insightspec-actor-cyberfabric-sre`

### 5.4 Layered Deploy Model and Customer Envs

#### Dual-purpose `<svc>.deploy` toggle

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-dual-purpose-toggle`

The umbrella chart **MUST** keep its infrastructure subcharts (`clickhouse`, `mariadb`, `redis`, `redpanda`) gated by per-service `<svc>.deploy: true|false` toggles so the same chart serves two install shapes:

- **Single-namespace fat install** (`<svc>.deploy: true` for every infra, used by `dev-up.sh` and by any external consumer who is fine running everything in one namespace): the umbrella renders MariaDB, ClickHouse, Redis, Redpanda **and** the app services together in the `insight` namespace.
- **Layered app-only install** (`<svc>.deploy: false` for every infra, used by gitops production): the umbrella renders the app services only into `insight`; infra services come from L2 in `insight-infra` (gitops Cyberfabric clusters) or from managed external endpoints / a separate team's namespace (Constructor Platform, external customers).

The same chart shape **MUST** render under both configurations; cross-namespace wiring uses the same `<svc>.host` / `<svc>.port` shape as Constructor Platform external mode.

**Rationale**: One chart, two operating modes — same templates exercise both, so a bug in app rendering is caught locally on `dev-up.sh` before it reaches a production cluster.

**Actors**: `cpt-insightspec-actor-platform-developer`, `cpt-insightspec-actor-cyberfabric-sre`, `cpt-insightspec-actor-customer-sre`

#### L0/L2/L3 layered architecture for gitops production

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-layered-architecture`

For Cyberfabric-operated clusters, the system **MUST** model the deploy as three independent layers, per the contract documented in [gitops SPEC §1.5](../gitops/README.md#15-layer-model):

- **L0 Bootstrap** — cluster prerequisites (sealed-secrets-controller, ingress-nginx, cert-manager) plus the `insight-infra` and `insight` namespaces. Cluster-scoped, runs once per cluster.
- **L2 System** — shared stateful infrastructure (MariaDB, ClickHouse, Redis, Redpanda, Redpanda Console, Airbyte, Argo Workflows). One Helm release per service in `insight-infra`. Each service can be self-hosted or replaced by a managed external endpoint without changing the umbrella's values surface — the L3 app values point at the actual host either way.
- **L3 App** — the umbrella chart, app services only, in the `insight` namespace. Pulled from `oci://ghcr.io/cyberfabric/charts/insight` pinned to `.insight-version`.

An L3 upgrade **MUST NOT** re-roll an L2 service; an L2 service upgrade **MUST NOT** re-roll L3 app pods. Cross-layer wiring **MUST** use Kubernetes DNS (`<release>.insight-infra.svc.cluster.local`) or explicit `<svc>.host` values.

**Rationale**: Stateful infra and the app have different upgrade cadences and risk profiles. Layering them as independent Helm releases means an app version bump never migrates a database; an infra patch never restarts the gateway. Layer separation also makes managed-external swaps (RDS, MSK, …) a per-service operational choice rather than a chart-level one.

**Actors**: `cpt-insightspec-actor-cyberfabric-sre`, `cpt-insightspec-actor-kubernetes`

#### Customer-named env model with confirm-token gating

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-customer-named-envs`

The gitops repo **MUST** name every production environment after the customer that owns it (`virtuozzo`, `constructor`, `acronis`, …) with no generic "prod" alias. Each customer install **MUST** live in its own `environments/<env>/` directory and be addressed via its own kube-context (`insight-<env>`). The Makefile **MUST** enforce a `PROTECTED_ENVS` allow-list for every customer-named env and **MUST** require an explicit `CONFIRM=yes-deploy-<env>` token on `make deploy` for envs in that list — each customer cluster requires its own per-env token (`yes-deploy-constructor`, `yes-deploy-acronis`, …) so muscle memory does not carry across customers.

**Rationale**: "prod" is ambiguous when there are five of them. Customer-named envs plus per-customer confirm tokens make wrong-cluster deploys structurally unlikely; the token has to be typed deliberately for each customer.

**Actors**: `cpt-insightspec-actor-cyberfabric-sre`

#### Two well-known namespaces per cluster

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-namespace-convention`

Every cluster targeted by gitops production **MUST** carry exactly two Insight-owned namespaces: `insight-infra` for L2 shared services and `insight` for the L3 umbrella release. The cluster's environment identity **MUST** live in the kube-context name and the gitops repo directory — not in the namespace. This matches the `dev-up.sh` local convention (everything in `insight`) and the external chart consumer's expectation of a single `insight` release name.

**Rationale**: Two well-known namespace names across every install make tooling, runbooks and `kubectl` commands reproducible across customer environments; the env identity stays out of the namespace string so the chart shape does not vary by env.

**Actors**: `cpt-insightspec-actor-cyberfabric-sre`, `cpt-insightspec-actor-customer-sre`

### 5.5 Developer Workflow

#### Dev wrapper for local bring-up

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-dev-wrapper`

The system **MUST** ship `dev-up.sh` (renamed from the legacy `up.sh`) that bootstraps a Kind/OrbStack cluster, builds backend images from source and loads them into the cluster, builds the frontend image from the sibling `insight-front` checkout (with `docker pull --platform` fallback) to avoid Apple Silicon arm64/amd64 manifest mismatches, applies `deploy/values-dev.yaml` automatically, installs Airbyte and Argo Workflows into the local `insight` namespace via dev-helper scripts, installs the umbrella chart with `<svc>.deploy: true` for the infra subcharts, and opens port-forwards for the common UIs (Frontend :8003, API Gateway :8080, Airbyte UI :8002, Airbyte API :8001, Argo UI :2746, ClickHouse HTTP :8123).

**Rationale**: The dev path must exercise the same umbrella chart as production so layout bugs are caught in dev; image build + Kind loading + dev overlays must be invisible to the developer.

**Actors**: `cpt-insightspec-actor-platform-developer`

#### Namespace parameterisation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-dev-namespace-param`

`dev-up.sh`, `dev-down.sh` and `init.sh` **MUST** honour an `INSIGHT_NAMESPACE` environment variable defaulting to `insight`, so that multiple concurrent dev environments can share a single Kind cluster by choosing distinct namespaces.

**Rationale**: Two parallel feature branches on the same cluster is a common developer need; hard-coded namespaces block that.

**Actors**: `cpt-insightspec-actor-platform-developer`

### 5.6 Multi-Tenant Deployment

#### Per-cluster, per-tenant isolation

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-tenant-isolation-boundary`

Each Cyberfabric-operated cluster **MUST** host exactly one Insight install (one customer per cluster) — tenant separation across customers is at the cluster boundary. Two installs on a single shared cluster (Constructor Platform tenants, dev wrapper concurrent runs) **MUST** be isolated by distinct namespaces, with Argo Workflows scoped via `controller.workflowNamespaces` and `controller.instanceID`. No ClusterRole or ClusterRoleBinding is created by L3 deploys; cross-namespace DNS is the only cross-namespace coupling.

**Rationale**: Cluster-per-customer is the gitops production model; namespace-per-tenant is the shared-cluster model. Both axes need to work without changing the chart shape.

**Actors**: `cpt-insightspec-actor-cyberfabric-sre`, `cpt-insightspec-actor-platform-operator`, `cpt-insightspec-actor-customer-sre`

### 5.7 Credential Hygiene

#### Empty-by-default credential fields

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-empty-credentials-default`

The canonical `charts/insight/values.yaml` **MUST** leave all credential fields empty (no `changeme`, no inline database URLs with passwords, no default admin passwords) and **MUST** rely on the fail-fast validator to reject installs that neither supply inline credentials nor declare an existing Secret.

**Rationale**: Default passwords that reach production are a frequent class of incident; failing fast is strictly better than succeeding silently.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-cyberfabric-sre`

#### Dev overlay isolation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-dev-overlay-isolation`

Eval / dev credentials **MUST** live in a separate file `deploy/values-dev.yaml` that is applied only by the dev wrapper, and **MUST NOT** appear anywhere in the canonical chart values or in any published artifact. Production credentials reach the cluster through sealed secrets fed from a corporate secret store (Passbolt in the gitops model), or through customer-owned Secrets pre-created in the namespace, never via committed values files.

**Rationale**: Keeps throwaway eval passwords out of the production code path by construction; keeps real credentials out of git by construction.

**Actors**: `cpt-insightspec-actor-platform-developer`, `cpt-insightspec-actor-cyberfabric-sre`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Multi-tenant isolation on a shared cluster

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-dep-tenant-isolation`

Two Insight installs on the same Kubernetes cluster in different namespaces **MUST NOT** observe each other's Kubernetes Secrets, ConfigMaps, Argo Workflow objects or WorkflowTemplate objects at the RBAC level granted by the L3 deploy.

**Threshold**: no cross-namespace RBAC binding created by an L3 deploy; Argo controllers scoped via `controller.workflowNamespaces` and `controller.instanceID`.

**Rationale**: Constructor Platform operates as a shared fabric and the dev wrapper supports parallel branches per developer — cross-tenant leakage would be a platform-level incident.

#### Fail-fast on misconfiguration

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-dep-fail-fast`

A chart render that is missing any of the following **MUST** abort during `helm template` or `helm install` with a human-readable message that names the missing field: `<dep>.host` for any infra with `<dep>.deploy: false`; `<dep>.passwordSecret.{name,key}` for any infra; for BYO mode, any required key in a pre-existing `insight-db-creds` Secret that is missing or empty (and, as a hardening, any password containing URL-reserved characters that would silently corrupt embedded DSNs); partially-configured OIDC (some but not all of `issuer` / `clientId` / `redirectUri`) when `apiGateway.authDisabled: false`.

**Threshold**: zero renders that reach the cluster with a missing required field; every such render aborted at template time.

**Rationale**: Runtime failures on a partially-installed cluster are an order of magnitude harder to diagnose than render-time errors.

#### Chart publish freshness

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-dep-chart-publish-freshness`

Every merge to `main` of `cyberfabric/insight` **MUST** publish a new umbrella chart tag to `oci://ghcr.io/cyberfabric/charts/insight` within 15 minutes of the merge commit. The gitops `auto_envs` poller **MUST** pick up the new tag within one hour (cron `0 * * * *`) and commit a `.insight-version` bump for envs in `auto_envs` (currently `[dev]`).

**Threshold**: p95 publish time ≤ 15 min from merge to OCI tag visible; poller lag ≤ 60 min from publish to gitops bump on auto-envs.

**Rationale**: The "one merge → one fix on dev" loop is the principal feedback channel for the platform team; longer than an hour breaks the dev/test rhythm.

### 6.2 NFR Exclusions

- **Install time target**: removed in this revision. The Deployment subsystem no longer ships an opinionated installer; install duration depends on the consumer's choice of tooling, the cluster's image pull bandwidth and the layered model's pre-existing L2 state. Time-to-Ready measurements live with the consumer (gitops Makefile timing for Cyberfabric SRE; `dev-up.sh` self-times for developers).
- **Availability target (REL-PRD-001)**: Not applicable because the Deployment subsystem produces a chart artifact and a dev wrapper, not a running service. The availability SLO of the running platform is defined in the Backend PRD.
- **Recovery targets RPO/RTO (REL-PRD-002)**: Not applicable because Deployment does not persist runtime state. Backup/restore of the data stores is defined separately; see Backend PRD and the Ingestion Layer PRD.
- **Performance response-time expectations (PERF-PRD-001)**: Not applicable because no user-facing request path lives inside the Deployment subsystem.
- **Accessibility (UX-PRD-002)**: Not applicable because this subsystem has no end-user UI; it is operator-facing CLI and YAML.
- **Internationalisation (UX-PRD-003)**: Not applicable because all operator-facing output is English and intended for SREs.
- **Offline capability (UX-PRD-004)**: Not applicable because chart distribution inherently requires registry connectivity; offline / air-gapped installs (image + chart pre-loaded into a customer registry) are a future consideration.
- **Inclusivity (UX-PRD-005)**: Not applicable because the audience is a narrow technical one — SREs and platform engineers.
- **Regulatory compliance (COMPL-PRD-001)**: Not applicable at this layer because the Deployment subsystem does not process personal data; regulatory obligations apply to the running platform and are captured in the Backend PRD.
- **Privacy by Design (SEC-PRD-005)**: Not applicable — no personal data flows through the chart artifact or the dev wrapper.
- **Safety (SAFE-PRD-001/002)**: Not applicable — software-only artifact pipeline with no physical side effects.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Umbrella chart values contract

- [ ] `p1` - **ID**: `cpt-insightspec-interface-dep-chart-values`

**Type**: Helm chart values schema (`charts/insight/values.yaml` + `values.schema.json`, documented in `charts/insight/README.md`).

**Stability**: unstable (pre-1.0 while the chart is at `version: 0.1.x`).

**Description**: The values contract that every consumer (Cyberfabric SRE in gitops, Constructor Platform, external customers, `dev-up.sh`) targets. It covers the `credentials` block (`autoGenerate`), the `global` block, the four infra blocks (ClickHouse, MariaDB, Redis, Redpanda) each with the unified flat shape (`deploy`, `host`, `port`, `database`, `username`, `passwordSecret`), the three mandatory app-service blocks (apiGateway, analyticsApi, frontend) plus the optional `identityResolution` (`deploy`-gated), and the `airbyte` + `ingestion.templates` blocks. The `<svc>.deploy` toggles drive the dual-purpose dev-vs-prod split documented in [§5.4](#54-layered-deploy-model-and-customer-envs).

**Breaking Change Policy**: minor version bump on the umbrella for additive fields; major version bump for removed or renamed values keys; the validator output must name any newly required field.

#### OCI artifact reference

- [ ] `p1` - **ID**: `cpt-insightspec-interface-dep-oci-artifact`

**Type**: Helm OCI repository.

**Stability**: stable URL (`oci://ghcr.io/cyberfabric/charts/insight`), per-tag artifacts are immutable.

**Description**: The single addressable artifact every consumer pulls. Tags are semver, one published per merge to `main` of `cyberfabric/insight`. The `charts/` URL segment is part of the GHCR package name (standard Helm-OCI behaviour). Subchart and app images live at `ghcr.io/cyberfabric/insight-<service>:<buildtag>`; their tags are referenced from the published chart's per-subchart `appVersion` field.

**Breaking Change Policy**: tags are immutable; the publishing CI does not overwrite. Retention policy on GHCR may delete old tags; consumers pinning a specific version SHOULD mirror to their own registry for long-term reproducibility.

### 7.2 External Integration Contracts

#### Airbyte consumer contract

- [ ] `p2` - **ID**: `cpt-insightspec-contract-dep-airbyte`

**Direction**: required from client (Insight consumes Airbyte's API).

**Protocol/Format**: HTTP/JSON on the Airbyte REST API; bearer token obtained via OAuth2 client_credentials at `/api/v1/applications/token` using `instance-admin-client-id` / `instance-admin-client-secret` from the `airbyte-auth-secrets` Secret created by the Airbyte chart. No JWT signing on our side. In gitops production, Airbyte runs as an L2 release in `insight-infra`; in `dev-up.sh` it runs in the local `insight` namespace; the chart reads `airbyte.apiUrl` either way.

**Compatibility**: pinned to Airbyte chart 1.8.5+ / app 1.8.5+ at the consumer side. Chart 1.9.x was intentionally skipped while its bundled app was 2.0.x-alpha. Version bumps happen in dedicated PRs with regression tests.

#### Argo Workflows consumer contract

- [ ] `p2` - **ID**: `cpt-insightspec-contract-dep-argo`

**Direction**: required from client (Insight's ingestion pipelines are Argo `WorkflowTemplate` / `CronWorkflow` objects).

**Protocol/Format**: Argo CRDs. The controller must watch the `insight` namespace (`controller.workflowNamespaces`) and identify this install via `controller.instanceID`. The CRDs must be present in the cluster for `ingestion.templates.enabled: true` to render successfully.

**Compatibility**: pinned to Argo Workflows chart 0.45.x at the consumer side.

#### OCI Helm registry contract

- [ ] `p1` - **ID**: `cpt-insightspec-contract-dep-oci-registry`

**Direction**: required from client (the chart consumer needs read access to GHCR).

**Protocol/Format**: OCI Helm pull (`helm pull oci://ghcr.io/cyberfabric/charts/insight --version <X.Y.Z>`).

**Compatibility**: Helm 3.14+ required for OCI chart pulls and registry authentication. GHCR-side publishing uses standard Helm-OCI, no `oras`-specific paths.

## 8. Use Cases

### 8.1 Eval install on a laptop

**ID**: `cpt-insightspec-usecase-dep-eval-install`

**Actors**: `cpt-insightspec-actor-platform-developer`, `cpt-insightspec-actor-customer-sre`

**Preconditions**: Docker Desktop or equivalent is running; kubectl and helm 3.14+ are installed; no Insight stack is running.

**Main Flow**:

1. Operator clones the repository and copies `.env.local.example` to `.env.local`.
2. Operator runs `./dev-up.sh --env local`.
3. Dev wrapper creates a Kind/OrbStack cluster, builds backend images, loads them, applies `deploy/values-dev.yaml` (which flips `<svc>.deploy: true` for the infra subcharts), and installs Airbyte and Argo Workflows in the local `insight` namespace via the dev-helper scripts.
4. Dev wrapper installs the umbrella chart in single-namespace fat mode; pods reach Ready.
5. Port-forwards open for Frontend, API Gateway, Airbyte UI, Argo UI, ClickHouse HTTP.
6. Operator opens http://localhost:8003 and sees the Insight UI.

**Postconditions**: all pods are Ready in namespace `insight`; eval credentials are in effect; Airbyte setup wizard is complete.

**Alternative Flows**:

- **Apple Silicon host**: dev-up detects arm64, falls back to `docker pull --platform linux/amd64` for the frontend image, Docker Desktop's QEMU emulation runs it.

### 8.2 Constructor Platform tenant install

**ID**: `cpt-insightspec-usecase-dep-platform-tenant`

**Actors**: `cpt-insightspec-actor-platform-operator`

**Preconditions**: Constructor Platform provides a shared ClickHouse / MariaDB / Redpanda / Airbyte reachable from the tenant namespace; Secrets with credentials are already provisioned; namespace is empty.

**Main Flow**:

1. Operator pre-creates `insight-db-creds` in the target namespace with the platform-issued passwords, then prepares an overlay values file that sets `credentials.autoGenerate: false`, `clickhouse.deploy: false`, `mariadb.deploy: false`, `redis.deploy: false`, `redpanda.deploy: false`, each with the matching flat `host` / `port` / `passwordSecret` block, pointing at the platform's services.
2. Operator runs `helm upgrade --install insight oci://ghcr.io/cyberfabric/charts/insight --version <X.Y.Z> --namespace <ns> -f overlay.yaml` (or wires the same artifact reference into their ArgoCD/Flux setup).
3. The umbrella's validator verifies every `<dep>.host` is present and every `<dep>.passwordSecret.{name,key}` resolves; `lookup` reads `insight-db-creds` and refuses to render with a missing or empty key.
4. Helm deploys application services that talk to the shared platform infra through the platform ConfigMap.

**Postconditions**: tenant Insight install is live without bundled stateful infra; shared-platform services carry tenant data isolated at the database level (outside this subsystem's concern).

**Alternative Flows**:

- **Missing Secret**: validator aborts render with a message naming the missing Secret.

### 8.3 Developer inner loop

**ID**: `cpt-insightspec-usecase-dep-dev-inner-loop`

**Actors**: `cpt-insightspec-actor-platform-developer`

**Preconditions**: developer has a checked-out repo, a working Kind/OrbStack cluster (or is about to create one via `dev-up.sh`), and is iterating on a backend service.

**Main Flow**:

1. Developer makes a code change in `src/backend/...`.
2. Developer runs `./dev-up.sh app` (or full `./dev-up.sh`), which rebuilds the affected image and loads it into Kind.
3. Helm upgrade runs against the local umbrella; the pod is rolled.
4. Developer re-opens the Frontend port-forward and exercises the change.
5. When done, developer runs `./dev-down.sh` to tear everything down.

**Postconditions**: clean cluster state at the end of the session; no leftover resources.

## 9. Acceptance Criteria

- [ ] `helm template insight oci://ghcr.io/cyberfabric/charts/insight --version <X.Y.Z>` with no overlay aborts with a readable message because OIDC and credentials are empty — zero successful renders of a misconfigured install.
- [ ] `helm template insight charts/insight -f deploy/values-dev.yaml` renders cleanly and produces every required Kubernetes object, including the three Argo `WorkflowTemplate` objects.
- [ ] On a merge to `main` of `cyberfabric/insight` that changes one service, the publish-chart workflow builds the image, bumps that subchart's `appVersion`, patch-bumps the umbrella, packages and pushes `oci://ghcr.io/cyberfabric/charts/insight:<new-semver>`, and auto-commits the version bumps back to `main`.
- [ ] `helm template` of the pulled chart confirms `image.tag` for the changed service equals the new build tag, others equal their previous tags.
- [ ] `./dev-up.sh` on a fresh laptop installs Airbyte, Argo and the umbrella into the local `insight` namespace; all pods reach Ready without manual intervention.
- [ ] Two concurrent installs in namespaces `insight-a` and `insight-b` on the same Kind cluster do not observe each other's Workflow objects.
- [ ] With `clickhouse.deploy: false` + a complete `clickhouse.host` / `.port` / `.passwordSecret` block, the resulting pods read from that external ClickHouse via the platform ConfigMap without modification to any subchart.
- [ ] `dev-up.sh` on Apple Silicon succeeds end-to-end without manual `docker pull --platform` calls; the `DEVLOG.md`-documented first-run failures do not regress.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Helm 3.14+ | Package manager for OCI chart pulls; required at every consumer. | p1 |
| Kubernetes 1.27+ | Target runtime; declared in the umbrella `kubeVersion`. | p1 |
| Airbyte chart 1.8.5+ | Data extraction engine; installed as a separate Helm release by every consumer that needs it. | p1 |
| Argo Workflows chart 0.45.x | Workflow engine for ingestion pipelines. | p1 |
| GHCR (`oci://ghcr.io/cyberfabric/charts/insight`) | Distribution target for the umbrella chart; written by Chart Publishing CI, read by every consumer. | p1 |
| Kind 0.22+ / OrbStack (dev only) | Local Kubernetes for the developer inner loop. | p2 |
| Docker Desktop / containerd with `kind load` support (dev only) | Image ingestion into Kind. | p2 |
| Bitnami Helm subcharts (MariaDB, Redis) via `bitnamilegacy` | Bundled-infra images still free after Bitnami's 2025 registry change. | p2 |
| Consumer-managed OIDC issuer | Required for any non-dev install (fail-fast validator enforces). | p1 |

## 11. Assumptions

- Cluster operators (Cyberfabric SRE for gitops, customer SREs for external installs) provide a working default StorageClass and an ingress controller; the Deployment subsystem does not provision either.
- Operators consuming the chart are comfortable with Helm values files, kubectl, and at least one of (helm, ArgoCD, Flux, Terraform Helm provider); the chart is not targeted at non-technical operators.
- The sibling repository `insight-front` (symlinked as `insight-front_symlink`) is present on developer machines for the dev wrapper's frontend build step.
- The bundled Airbyte and Argo Workflows versions remain viable for the next release cycle; upgrades to newer minors are handled in dedicated PRs with regression tests over ingestion workflows.
- On a shared cluster, tenant isolation is acceptable at the Kubernetes namespace boundary — workloads within a tenant namespace are mutually trusted. On a Cyberfabric-operated cluster, tenant isolation is at the cluster boundary (one customer per cluster).
- The Constructor Platform provides stable Secret resource references; tenants receive them out-of-band (not created by the consumer's chart install).
- The `infra/insight-gitops` repo (private, on internal GitLab) exists and is operationally maintained by Cyberfabric SRE per its own [SPEC](../gitops/README.md); the public deployment artifacts in this repo do not depend on its content.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Chart Publishing CI auto-commit-back fails on branch protection. | A merge that should publish a new umbrella tag publishes the chart but fails to bump `Chart.yaml` on `main`, leaving the repo state out of sync with what was published. | Track in repo settings: fine-grained PAT in `RELEASE_PUSH_PAT` with bypass on protected branch, or a GitHub App with bypass rights. Until either is in place, the auto-commit step is replayed manually after merge. |
| Inline infra passwords previously had to be duplicated into app-service DSNs. | Drift between infra password and DSN produced a silently-broken install. | Resolved: `credentials.autoGenerate=true` writes `insight-db-creds` once and the umbrella derives all app-service Secrets (`insight-analytics-api-config`, `insight-identity-resolution-config`) from the same passwords. BYO mode reads the customer-supplied `insight-db-creds` instead. |
| Frontend image is `linux/amd64` only — Apple Silicon hosts rely on QEMU emulation or local rebuild. | Slow first pull and occasional emulation bugs on dev machines. | Dev wrapper builds the frontend from source as a workaround; infra team to publish multi-arch images. |
| Identity Resolution subchart ships as MVP stub that crashloops on empty bronze. | If operator flips `identityResolution.deploy: true` before any BambooHR sync, the release looks broken. | Keep default `identityResolution.deploy: false`; document the prerequisite in README; surface a clearer error message in the service itself (Backend concern). |
| Airbyte chart 1.9.x was deliberately skipped because its bundled app 2.0.x-alpha is not production-grade. | Consumer asking for 1.9 gets a "no". | Document the policy in the Airbyte README; revisit when 2.0 GA ships. |
| Bitnami's late-2025 registry change means the MariaDB / Redis subcharts rely on `bitnamilegacy` + `global.security.allowInsecureImages`. | If Bitnami deprecates `bitnamilegacy`, both subcharts break. | Monitor Bitnami's policy; plan a migration to a vendored or self-hosted registry; enterprise customers are expected to use their own internal registry. |
| GHCR retention may delete old umbrella tags. | A gitops env pinned to an old version can lose its chart artifact. | Cyberfabric SRE policy: mirror long-lived production pins to a self-hosted registry; document the procedure in the gitops SPEC follow-ups (`§8`). |
| Chart artifact is not yet signed. | A compromised intermediate could publish a malformed chart that the gitops poller would pick up. | Track as an open item in the gitops SPEC ([§8](../gitops/README.md#8-open-items)); plan cosign signing at publish time and verification before `make deploy`. |
