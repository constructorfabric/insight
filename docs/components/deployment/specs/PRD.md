---
status: proposed
date: 2026-04-23
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
   - [5.3 Canonical Installer](#53-canonical-installer)
   - [5.4 GitOps Path](#54-gitops-path)
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
   - [8.2 Production install on a customer Kubernetes cluster](#82-production-install-on-a-customer-kubernetes-cluster)
   - [8.3 Constructor Platform tenant install](#83-constructor-platform-tenant-install)
   - [8.4 GitOps-managed install via ArgoCD](#84-gitops-managed-install-via-argocd)
   - [8.5 Developer inner loop](#85-developer-inner-loop)
9. [9. Acceptance Criteria](#9-acceptance-criteria)
10. [10. Dependencies](#10-dependencies)
11. [11. Assumptions](#11-assumptions)
12. [12. Risks](#12-risks)

## 1. Overview

### 1.1 Purpose

The Deployment subsystem is the unit of distribution for the Insight platform. It packages the Insight umbrella Helm chart, the two supporting engines (Airbyte and Argo Workflows) and a set of installers and GitOps manifests so that a single command (or a single ArgoCD Application) brings the full platform up inside a customer Kubernetes cluster. It also provides the developer bring-up wrapper that builds images from source and loads them into a local Kind cluster.

The subsystem does not ship product functionality on its own — it composes the application services (API Gateway, Analytics API, Frontend, optional Identity Resolution) with their required infrastructure (ClickHouse, MariaDB, Redis, Redpanda, Airbyte, Argo) into a releasable artifact and enforces the contracts between them (single-namespace model, external-mode infra contracts, fail-fast validation, mandatory OIDC in production).

### 1.2 Background / Problem Statement

Before this subsystem the Insight stack was brought up through ad-hoc Docker Compose files, a free-form `up.sh` and per-service helm charts applied one at a time. That worked for the founding team but could not be handed to a customer or to the Constructor Platform SRE group: there was no canonical Helm artifact, no validated wiring between the application services and their infrastructure, no opinionated story for where Airbyte and Argo Workflows live, and no way to describe the product as a single GitOps-managed entity.

Two concrete pain points drove this work. First, enterprise customers that standardise on ArgoCD need one Application manifest that owns the whole platform — not seven. Second, the Constructor Platform (internal multi-product infrastructure fabric) shares ClickHouse, MariaDB and Redpanda across products, and expects each product to consume those services via declared external contracts with explicit credential Secrets. Neither use case was supported by the pre-existing bring-up scripts.

The third driver is reproducibility for the development team itself: a developer joining the project should be able to clone the repo, run one script, and end up with a live stack that mirrors the production topology — so that layout bugs are caught in dev rather than in a customer environment.

### 1.3 Goals (Business Outcomes)

- Reduce the time from "clean Kubernetes cluster" to "Insight UI reachable" to under 30 minutes for a customer SRE following the canonical installer, measured end-to-end on a fresh Kind or managed cluster.
- Enable Constructor Platform onboarding by allowing each infra dependency to be flipped from bundled to external via a single `<dep>.deploy: false` toggle plus the same flat `host` / `port` / `passwordSecret` fields the bundled mode reads, so a shared-platform tenant install reuses the platform's ClickHouse / MariaDB / Redpanda without code changes.
- Ship a single `Application` (App-of-Apps) manifest that enterprise ArgoCD users apply once to reconcile the entire stack, so upgrade = git commit.
- Keep developer inner-loop under 10 minutes from `dev-up.sh` to a usable cluster with locally built images, so platform changes can be tested against a realistic topology before review.
- Prevent accidental shipping of default passwords or placeholder secrets by failing `helm install` fast when credentials are empty and no external Secret is declared.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Umbrella chart | The `charts/insight/` Helm chart that aggregates all Insight subcharts (infra + app services + ingestion templates) via Chart.yaml dependencies. |
| Canonical installer | The orchestrator script `deploy/scripts/install.sh` and its three step scripts (`install-airbyte.sh`, `install-argo.sh`, `install-insight.sh`) that customers are expected to run. |
| Dev wrapper | `dev-up.sh` (and `dev-down.sh`) — bring-up scripts that build images from source, create a local Kind cluster, and apply the same installers with dev overlays. |
| Single-namespace model | Deployment topology in which Airbyte, Argo Workflows and the Insight umbrella are three separate Helm releases that all target the same Kubernetes namespace. Multi-tenant separation on a shared cluster is done via distinct namespaces, one per install. |
| External mode | State of an infra dependency where `<dep>.deploy: false`. The umbrella does not run the bundled subchart; consumers read the same flat `<dep>.host`, `<dep>.port` and `<dep>.passwordSecret` fields and the Secret is provided by the operator (or platform). |
| Constructor Platform | Shared multi-product infrastructure fabric operated by the vendor. It provides ClickHouse, MariaDB, Redpanda and identity services that tenant products consume via external-mode contracts. |
| App-of-Apps | ArgoCD pattern in which one parent Application manifest owns four child Applications (Airbyte, Argo, Argo RBAC, Insight) so sync-wave annotations enforce ordering across them. |
| Platform ConfigMap | The single `{release}-platform` ConfigMap emitted by the umbrella that contains resolved infra coordinates (CLICKHOUSE_URL, MARIADB_HOST, AIRBYTE_API_URL, …). Pods consume it via `envFrom`. |
| Eval credentials | Throwaway passwords in `deploy/values-dev.yaml` used only by dev bring-up and short-lived eval clusters; never shipped to production. |

## 2. Actors

### 2.1 Human Actors

#### Customer SRE

**ID**: `cpt-insightspec-actor-customer-sre`

**Role**: Operator on the customer side who installs, upgrades and rolls back the Insight stack on a Kubernetes cluster they own.
**Needs**: A single reproducible install command, explicit documentation for external-mode overrides, a path to roll back failed upgrades, and clear failure messages when required values are missing.

#### Constructor Platform Operator

**ID**: `cpt-insightspec-actor-platform-operator`

**Role**: Internal operator who onboards Insight as a tenant of the Constructor Platform, wiring it to the shared ClickHouse / MariaDB / Redpanda.
**Needs**: Per-infra `deploy` flags plus a single flat block (`host`, `port`, `database`, `username`, `passwordSecret`) that the chart reads identically whether the dependency is bundled or external, and a validator that fails fast when any of those are missing.

#### Enterprise ArgoCD Administrator

**ID**: `cpt-insightspec-actor-argocd-admin`

**Role**: Enterprise platform team member who manages ArgoCD and expects the product to be installable and upgradable declaratively from Git.
**Needs**: One entry-point manifest that owns the whole stack, chart references that work from an OCI registry, a values file reference pattern that supports per-environment overrides, and well-behaved sync-wave ordering between infra and application releases.

#### Platform Developer

**ID**: `cpt-insightspec-actor-platform-developer`

**Role**: Engineer on the Insight team iterating on the services, charts or ingestion code.
**Needs**: A single dev wrapper that builds images locally, bootstraps a Kind cluster, applies dev overlays, port-forwards the relevant services, and reuses the same installer scripts that customers run so bugs show up before release.

### 2.2 System Actors

#### Kubernetes Cluster

**ID**: `cpt-insightspec-actor-kubernetes`

**Role**: Target runtime. The Deployment subsystem targets Kubernetes 1.27+ (declared in the umbrella's `Chart.yaml` via `kubeVersion`), served either by Kind locally or by a customer-owned production cluster.

#### Helm

**ID**: `cpt-insightspec-actor-helm`

**Role**: Package manager used by all three release paths (canonical installer, GitOps, dev wrapper). The umbrella ships as a Helm chart; Airbyte and Argo Workflows are their upstream Helm charts pinned by version.

#### ArgoCD

**ID**: `cpt-insightspec-actor-argocd`

**Role**: GitOps controller (2.6+, for multi-source support) used in the enterprise path. Consumes the Application manifests under `deploy/gitops/`, resolves values via the `$values` multi-source pattern, and reconciles the cluster to match Git.

#### Argo Workflows Controller

**ID**: `cpt-insightspec-actor-argo-workflows`

**Role**: Engine that executes the ingestion `WorkflowTemplates` emitted by the umbrella. Installed as a separate Helm release in the same namespace; scoped to the install via `controller.instanceID` and `controller.workflowNamespaces`.

#### Airbyte Engine

**ID**: `cpt-insightspec-actor-airbyte-engine`

**Role**: Data extraction engine. Installed as a separate Helm release in the same namespace, pinned to chart 1.8.5 / app 1.8.5. The canonical installer completes Airbyte's one-time setup wizard via its REST API so the UI is usable on first visit.

#### OCI / Git Artifact Registry

**ID**: `cpt-insightspec-actor-artifact-registry`

**Role**: Source of record for the umbrella chart (`oci://ghcr.io/cyberfabric/charts/insight`) and application images (`ghcr.io/cyberfabric/insight-*`). The installer accepts both a local chart path and an OCI reference.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- Target Kubernetes version: 1.27 or newer (declared via `kubeVersion: ">=1.27.0-0"` in the umbrella `Chart.yaml`).
- Helm 3.14+ required for OCI chart pulls and the multi-source `$values` ArgoCD pattern.
- ArgoCD 2.6+ required on the GitOps path (multi-source support).
- Docker Desktop / Rancher Desktop / Podman with working containerd image load required on the developer path (the dev wrapper uses `kind load docker-image`).
- Bitnami chart dependencies (MariaDB, Redis) are pinned to the `bitnamilegacy` registry variants with `global.security.allowInsecureImages: true`, because Bitnami moved free images off `docker.io/bitnami/*` in 2025.
- Frontend image is currently published as `linux/amd64` only. On Apple Silicon hosts the dev wrapper rebuilds the frontend from the sibling `insight-front` checkout rather than pulling the upstream image; production installs rely on Docker Desktop's QEMU emulation.
- The umbrella chart assumes release name `insight` for its internal DNS references inside `values.yaml`. Using a non-default release name requires overriding the affected URL fields.
- The dev wrapper targets Kind 0.22+; cluster bootstrapping uses a fixed cluster name `insight` to match hard-coded port mappings.

## 4. Scope

### 4.1 In Scope

- The Insight umbrella Helm chart at `charts/insight/` with eight declared dependencies (ClickHouse, MariaDB, Redis, Redpanda, API Gateway, Analytics API, Frontend, Identity Resolution).
- The service-resolution helper library (`templates/_helpers.tpl`) that returns the same values whether a dependency is bundled or external, and the `insight.validate` template that fails rendering on missing required fields.
- The single `{release}-platform` ConfigMap that exposes resolved infra coordinates to every pod in the namespace via `envFrom`.
- Argo `WorkflowTemplate` emission as first-class Helm templates under `charts/insight/templates/ingestion/*.yaml`, gated by `ingestion.templates.enabled` and consuming umbrella helpers (`insight.clickhouse.fqdn`, `insight.airbyte.url`, …) directly via `include`.
- Airbyte bring-up assets: pinned chart version 1.8.5 + app 1.8.5, curated values file under `deploy/airbyte/`, installer script that completes the setup wizard via API.
- Argo Workflows bring-up assets: pinned chart version, curated values, supplemental RBAC with placeholder substitution, installer that configures `controller.instanceID` and `workflowNamespaces` per install.
- Orchestrator installer `deploy/scripts/install.sh` (Airbyte → Argo → Insight) with per-step skip flags.
- GitOps manifests under `deploy/gitops/` — four Applications plus an App-of-Apps root, using the ArgoCD multi-source `$values` pattern for values file references.
- Developer bring-up wrappers `dev-up.sh` / `dev-down.sh` / `init.sh`, parameterised by `INSIGHT_NAMESPACE`, with Kind bootstrap, image build + `kind load`, and port-forwards for the common UIs.
- Dev-only credential overlay `deploy/values-dev.yaml` (throwaway passwords) applied automatically by `dev-up.sh`.
- The DEVLOG.md that records the first-run debugging narrative so future customers and dev-up users can resolve the same twelve issues without rediscovering them.

### 4.2 Out of Scope

- Release automation (tag → build images → package chart → push OCI): versions are pinned in chart metadata but there is no CI pipeline that publishes the chart yet. Flagged under Risks.
- Multi-architecture (linux/arm64) frontend image publication.
- Bidirectional sync between the umbrella-managed `insight-db-creds` Secret and a customer-supplied secret-management system (Vault, AWS Secrets Manager, External Secrets Operator). Customers integrating with such systems pre-create `insight-db-creds` themselves (the chart auto-detects BYO via absence of the `app.kubernetes.io/managed-by=Helm` label and skips its own Secret-template emission, so the customer's labeling-or-no-labeling on the Secret is preserved as-is); or they accept the auto-generated values and mirror them outwards by their own means.
- Cluster provisioning (creating the customer's Kubernetes cluster, setting up a StorageClass, installing ingress-nginx on a production cluster). The dev wrapper bootstraps Kind for local work; production installs assume a working cluster with a default StorageClass and an ingress controller already in place.
- Backup, restore, and disaster-recovery workflows for the bundled stateful services (ClickHouse, MariaDB). Mentioned in the Backend PRD; not owned by Deployment.
- Identity Provider (OIDC) provisioning. The deployment contract requires OIDC credentials as input; standing up an IdP is the customer's responsibility.
- Customer-facing documentation portal. Internal README files and DEVLOG.md are in scope; hosted docs are not.

## 5. Functional Requirements

### 5.1 Umbrella Chart Packaging

#### Single umbrella distributable

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-umbrella-chart`

The system **MUST** ship a single Helm umbrella chart named `insight` that aggregates the four infrastructure subcharts (ClickHouse, MariaDB, Redis, Redpanda) and the four application subcharts (API Gateway, Analytics API, Frontend, Identity Resolution) as declared dependencies in `Chart.yaml`, so that a single `helm install insight charts/insight` renders every Kubernetes object that the platform requires.

**Rationale**: A single artifact is what enterprise customers can consume, version, roll back and audit. Seven independent releases are not a product.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-argocd-admin`

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

Each infrastructure dependency in the umbrella (ClickHouse, MariaDB, Redis, Redpanda) **MUST** expose a single unified shape — `<dep>.deploy: true/false` plus flat `host` / `port` / (where applicable) `database` / `username` / `passwordSecret.{name,key}` — read identically by consumers whether the dependency is bundled (umbrella runs the subchart) or external (umbrella does not run the subchart and the operator points the same fields at a platform-provided instance).

**Rationale**: Constructor Platform tenant installs must reuse the platform's shared ClickHouse / MariaDB / Redpanda — the umbrella cannot assume every install bundles its own.

**Actors**: `cpt-insightspec-actor-platform-operator`

#### Fail-fast validation of external contracts

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-fail-fast-validation`

The umbrella chart **MUST** invoke an `insight.validate` template during rendering that fails rendering with a readable message whenever `<dep>.deploy: false` is used without `<dep>.host`, whenever any `<dep>.passwordSecret.name` or `.key` is missing, whenever a pre-existing `insight-db-creds` Secret is present but a required key is missing or empty (BYO mode), or whenever `apiGateway.authDisabled: false` is set with neither `apiGateway.oidc.existingSecret` nor all three of `issuer` + `clientId` + `redirectUri` populated together.

**Rationale**: Silent defaults or partial configuration produces clusters that install cleanly but fail at runtime — by which time the operator has already lost access to the diagnostic output.

**Actors**: `cpt-insightspec-actor-platform-operator`, `cpt-insightspec-actor-customer-sre`

#### Helper-based service resolution

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-service-resolution-helpers`

The umbrella chart **MUST** resolve every infra host, port, FQDN and URL through named helpers in `_helpers.tpl` (rather than template-time string concatenation) that return the internal cluster-DNS name when a dependency is bundled and the externally-provided host verbatim when it is external, without appending the cluster-DNS suffix to a hostname that already contains a dot.

**Rationale**: Prevents `clickhouse.example.com.insight.svc.cluster.local` mangling in external mode and keeps rename refactors to a single file.

**Actors**: `cpt-insightspec-actor-platform-developer`

### 5.3 Canonical Installer

#### Three-step orchestrator

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-canonical-installer`

The system **MUST** ship a single entry-point installer `deploy/scripts/install.sh` that installs Airbyte, Argo Workflows and the Insight umbrella in that order against the same Kubernetes namespace, and **MUST** honour `SKIP_AIRBYTE=1`, `SKIP_ARGO=1`, `SKIP_INSIGHT=1` environment flags so that each step can be skipped independently when an upstream platform already provides it.

**Rationale**: "One command" is the expected customer experience for an enterprise product; skip flags are mandatory for Constructor Platform tenants where Airbyte or Argo may already exist.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-platform-operator`

#### Idempotent step scripts

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-idempotent-installs`

Each step script (`install-airbyte.sh`, `install-argo.sh`, `install-insight.sh`) **MUST** be safe to re-run without side effects on an already-installed stack, by invoking `helm upgrade --install` with a consistent set of values files.

**Rationale**: Operators expect to re-run installers to apply configuration changes; partial failures must be resumable.

**Actors**: `cpt-insightspec-actor-customer-sre`

#### Chart source switch (local vs OCI)

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-chart-source-switch`

`install-insight.sh` **MUST** accept `CHART_SOURCE=local` (a path to `charts/insight` in a checkout) or `CHART_SOURCE=oci` (an OCI reference, default `oci://ghcr.io/cyberfabric/charts/insight`) and **MUST** require an explicit `INSIGHT_VERSION` in OCI mode.

**Rationale**: Enterprise customers consume the chart from OCI; developers and GitOps dry-runs consume it from a local checkout. One installer must cover both.

**Actors**: `cpt-insightspec-actor-customer-sre`, `cpt-insightspec-actor-platform-developer`

#### Layered values files

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-layered-values`

`install-insight.sh` **MUST** accept a colon-separated `INSIGHT_VALUES_FILES` environment variable, applied in left-to-right order (later files override earlier ones), in addition to back-compatible single-file `INSIGHT_VALUES`, so that environments layer base + overlay + secret files without bespoke scripting.

**Rationale**: Enterprise deployments invariably stack a base values file + environment overlay + secret overlay; supporting only one `-f` forces operators to pre-merge.

**Actors**: `cpt-insightspec-actor-customer-sre`

#### Airbyte setup-wizard automation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-airbyte-setup`

`install-airbyte.sh` **MUST** invoke Airbyte's `POST /api/v1/instance_configuration/setup` endpoint with a server-minted access token after the chart reaches Ready, so that the UI is fully usable on first visit without a manual setup wizard.

**Rationale**: Airbyte 1.5.x+ simple-auth mode leaves the instance half-initialised if the wizard is closed; this blocks first-use.

**Actors**: `cpt-insightspec-actor-customer-sre`

#### Argo controller scoping

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-argo-instance-id`

`install-argo.sh` **MUST** set `controller.workflowNamespaces[0]` to the release namespace and **MUST** enable `controller.instanceID.enabled=true` with an explicit `controller.instanceID.explicitID=$RELEASE-$NAMESPACE`, so that two Insight installs on the same cluster never observe each other's Workflow objects.

**Rationale**: Without `instanceID` scoping, tenant A's Argo controller can pick up tenant B's workflows on a shared cluster.

**Actors**: `cpt-insightspec-actor-platform-operator`

### 5.4 GitOps Path

#### App-of-Apps canonical entry point

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-gitops-app-of-apps`

The system **MUST** ship an App-of-Apps root manifest (`deploy/gitops/root-app.yaml`) that owns four child Applications (Airbyte, Argo Workflows, Argo RBAC, Insight), so that ArgoCD sync-wave annotations enforce ordering between infra (wave 0) and the umbrella (wave 1).

**Rationale**: Sync-wave ordering only works between child Applications of a parent Application; directly-applied sibling Applications are not ordered. This is the only supported GitOps path.

**Actors**: `cpt-insightspec-actor-argocd-admin`

#### Multi-source values references

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-gitops-multi-source`

Each ArgoCD `Application` in `deploy/gitops/` **MUST** reference its values file through the multi-source `$values` pattern (chart source + `$values` repo source + `sources[].ref` binding), so that the same values file in Git is consumed identically by ArgoCD and by the imperative installer.

**Rationale**: Single source of truth for values — imperative `helm -f file` and declarative ArgoCD `valueFiles: [$values/file]` must render identical manifests.

**Actors**: `cpt-insightspec-actor-argocd-admin`

### 5.5 Developer Workflow

#### Dev wrapper for local bring-up

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-dev-wrapper`

The system **MUST** ship `dev-up.sh` (renamed from the legacy `up.sh`) that bootstraps a Kind cluster, builds backend images from source and loads them into the cluster, builds the frontend image from the sibling `insight-front` checkout (with `docker pull --platform` fallback) to avoid Apple Silicon arm64/amd64 manifest mismatches, applies `deploy/values-dev.yaml` automatically via `INSIGHT_VALUES_FILES`, merges `deploy/argo/values-dev.yaml` when `DEV_MODE=1`, and opens port-forwards for the common UIs (Frontend :8003, API Gateway :8080, Airbyte UI :8002, Airbyte API :8001, Argo UI :2746, ClickHouse HTTP :8123).

**Rationale**: The dev path must exercise the same installers as production so layout bugs are caught in dev; image build + Kind loading + dev overlays must be invisible to the developer.

**Actors**: `cpt-insightspec-actor-platform-developer`

#### Namespace parameterisation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-dev-namespace-param`

`dev-up.sh`, `dev-down.sh` and `init.sh` **MUST** honour an `INSIGHT_NAMESPACE` environment variable defaulting to `insight`, so that multiple concurrent dev environments can share a single Kind cluster by choosing distinct namespaces.

**Rationale**: Two parallel feature branches on the same cluster is a common developer need; hard-coded namespaces block that.

**Actors**: `cpt-insightspec-actor-platform-developer`

### 5.6 Multi-Tenant Deployment

#### Single-namespace deployment model

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-single-namespace-model`

The system **MUST** deploy Airbyte, Argo Workflows and the Insight umbrella as three separate Helm releases that all target the same Kubernetes namespace (default `insight`), and **MUST** achieve multi-tenant separation on a shared cluster through distinct namespaces per install, not through cross-namespace DNS or secret mirroring.

**Rationale**: Same-namespace DNS is simpler, avoids cross-namespace RBAC, and makes tenant isolation a single-dimension namespace choice rather than a matrix.

**Actors**: `cpt-insightspec-actor-platform-operator`, `cpt-insightspec-actor-customer-sre`

### 5.7 Credential Hygiene

#### Empty-by-default credential fields

- [ ] `p1` - **ID**: `cpt-insightspec-fr-dep-empty-credentials-default`

The canonical `charts/insight/values.yaml` **MUST** leave all credential fields empty (no `changeme`, no inline database URLs with passwords, no default admin passwords) and **MUST** rely on the fail-fast validator to reject installs that neither supply inline credentials nor declare an existing Secret.

**Rationale**: Default passwords that reach production are a frequent class of incident; failing fast is strictly better than succeeding silently.

**Actors**: `cpt-insightspec-actor-customer-sre`

#### Dev overlay isolation

- [ ] `p2` - **ID**: `cpt-insightspec-fr-dep-dev-overlay-isolation`

Eval / dev credentials **MUST** live in a separate file `deploy/values-dev.yaml` that is applied only by the dev wrapper via `INSIGHT_VALUES_FILES`, and **MUST NOT** appear anywhere in the canonical chart values or the GitOps manifests.

**Rationale**: Keeps throwaway eval passwords out of the production code path by construction.

**Actors**: `cpt-insightspec-actor-platform-developer`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Install time target

- [ ] `p2` - **ID**: `cpt-insightspec-nfr-dep-install-time`

A fresh install on a typical development laptop (8 CPU cores, 16 GiB RAM, Kind cluster) **SHOULD** reach a state where all pods are Ready within 15 minutes when run as `dev-up.sh` with cold image pulls, and within 25 minutes when the canonical installer runs against a fresh production-class cluster with external-mode infra declared.

**Threshold**: p50 ≤ 15 minutes for dev-up with cold cache; p95 ≤ 25 minutes for canonical installer against a fresh cluster.

**Rationale**: Eval experience and developer inner-loop cost tie directly to install time.

#### Multi-tenant isolation on a shared cluster

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-dep-tenant-isolation`

Two Insight installs on the same Kubernetes cluster in different namespaces **MUST NOT** observe each other's Kubernetes Secrets, ConfigMaps, Argo Workflow objects or WorkflowTemplate objects at the RBAC level granted by the installers.

**Threshold**: no cross-namespace RBAC binding created by any installer; Argo controllers scoped via `controller.workflowNamespaces` and `controller.instanceID`.

**Rationale**: Constructor Platform operates as a shared fabric — cross-tenant leakage would be a platform-level incident.

#### Fail-fast on misconfiguration

- [ ] `p1` - **ID**: `cpt-insightspec-nfr-dep-fail-fast`

An install that is missing any of the following **MUST** abort during `helm template` or `helm install` with a human-readable message that names the missing field: `<dep>.host` for any infra with `<dep>.deploy: false`; `<dep>.passwordSecret.{name,key}` for any infra; for BYO mode, any required key in a pre-existing `insight-db-creds` Secret that is missing or empty (and, as a hardening, any password containing URL-reserved characters that would silently corrupt embedded DSNs); partially-configured OIDC (some but not all of `issuer` / `clientId` / `redirectUri`) when `apiGateway.authDisabled: false`.

**Threshold**: zero installs that reach the cluster with a missing required field; every such install aborted at render time.

**Rationale**: Runtime failures on a partially-installed cluster are an order of magnitude harder to diagnose than render-time errors.

### 6.2 NFR Exclusions

- **Availability target (REL-PRD-001)**: Not applicable because the Deployment subsystem is a one-shot installer, not a running service. The availability SLO of the running platform is defined in the Backend PRD.
- **Recovery targets RPO/RTO (REL-PRD-002)**: Not applicable because Deployment does not persist runtime state. Backup/restore of the data stores is defined separately; see Backend PRD and the Ingestion Layer PRD.
- **Performance response-time expectations (PERF-PRD-001)**: Not applicable because no user-facing request path lives inside the Deployment subsystem.
- **Accessibility (UX-PRD-002)**: Not applicable because this subsystem has no end-user UI; it is operator-facing CLI and YAML.
- **Internationalisation (UX-PRD-003)**: Not applicable because all operator-facing output is English and intended for SREs.
- **Offline capability (UX-PRD-004)**: Not applicable because installs inherently require cluster connectivity; offline installs (air-gapped customer clusters) are a future consideration.
- **Inclusivity (UX-PRD-005)**: Not applicable because the audience is a narrow technical one — SREs and platform engineers.
- **Regulatory compliance (COMPL-PRD-001)**: Not applicable at this layer because the Deployment subsystem does not process personal data; regulatory obligations apply to the running platform and are captured in the Backend PRD.
- **Privacy by Design (SEC-PRD-005)**: Not applicable — no personal data flows through install scripts or chart renders.
- **Safety (SAFE-PRD-001/002)**: Not applicable — software-only install pipeline with no physical side effects.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### Umbrella chart values contract

- [ ] `p1` - **ID**: `cpt-insightspec-interface-dep-chart-values`

**Type**: Helm chart values schema (`charts/insight/values.yaml` + `values.schema.json`).

**Stability**: unstable (pre-1.0 while the chart is at `version: 0.1.0`).

**Description**: The values contract that customers and GitOps overlays target. It covers the `credentials` block (`autoGenerate`), the `global` block, the four infra blocks (ClickHouse, MariaDB, Redis, Redpanda) each with the unified flat shape (`deploy`, `host`, `port`, `database`, `username`, `passwordSecret`), the three mandatory app-service blocks (apiGateway, analyticsApi, frontend) plus the optional `identityResolution` (`deploy`-gated), and the `airbyte` + `ingestion.templates` blocks.

**Breaking Change Policy**: minor version bump for additive fields; major version bump for removed or renamed values keys; the validator output must name any newly required field.

### 7.2 External Integration Contracts

#### Airbyte consumer contract

- [ ] `p2` - **ID**: `cpt-insightspec-contract-dep-airbyte`

**Direction**: required from client (Insight consumes Airbyte's API and shares its namespace).

**Protocol/Format**: HTTP/JSON on the Airbyte REST API; server-signed JWT obtained from the `airbyte-auth-secrets` Secret (`jwt-signature-secret` key) created in the shared namespace by the Airbyte chart.

**Compatibility**: pinned to Airbyte chart 1.8.5 / app 1.8.5. Chart 1.9.x intentionally skipped because its bundled app is 2.0.x-alpha. Upgrades happen in dedicated PRs with regression tests.

#### Argo Workflows consumer contract

- [ ] `p2` - **ID**: `cpt-insightspec-contract-dep-argo`

**Direction**: required from client (Insight's ingestion pipelines are Argo `WorkflowTemplate` / `CronWorkflow` objects).

**Protocol/Format**: Argo CRDs. The controller must watch the release namespace (`controller.workflowNamespaces`) and identify this install via `controller.instanceID`.

**Compatibility**: pinned to Argo Workflows chart 0.45.x.

#### ArgoCD consumer contract

- [ ] `p2` - **ID**: `cpt-insightspec-contract-dep-argocd`

**Direction**: required from client (the enterprise GitOps path consumes ArgoCD).

**Protocol/Format**: `argoproj.io/v1alpha1 Application` manifests with multi-source spec (`$values` pattern).

**Compatibility**: ArgoCD 2.6+ required for multi-source. Downgrading to 2.5 breaks the values file references.

## 8. Use Cases

### 8.1 Eval install on a laptop

**ID**: `cpt-insightspec-usecase-dep-eval-install`

**Actors**: `cpt-insightspec-actor-platform-developer`, `cpt-insightspec-actor-customer-sre`

**Preconditions**: Docker Desktop or equivalent is running; kubectl and helm are installed; no Insight stack is running.

**Main Flow**:

1. Operator clones the repository and copies `.env.local.example` to `.env.local`.
2. Operator runs `./dev-up.sh --env local`.
3. Dev wrapper creates a Kind cluster, builds backend images, loads them, applies `deploy/values-dev.yaml` automatically.
4. Canonical installer runs underneath: Airbyte → Argo → Insight.
5. Port-forwards open for Frontend, API Gateway, Airbyte UI, Argo UI, ClickHouse HTTP.
6. Operator opens http://localhost:8003 and sees the Insight UI.

**Postconditions**: all pods are Ready in namespace `insight`; eval credentials are in effect; Airbyte setup wizard is complete.

**Alternative Flows**:

- **Apple Silicon host**: dev-up detects arm64, falls back to `docker pull --platform linux/amd64` for the frontend image, Docker Desktop's QEMU emulation runs it.

### 8.2 Production install on a customer Kubernetes cluster

**ID**: `cpt-insightspec-usecase-dep-production-install`

**Actors**: `cpt-insightspec-actor-customer-sre`

**Preconditions**: a production Kubernetes cluster with a default StorageClass, an ingress controller and DNS configured; kubectl context points at that cluster; an OIDC issuer + client credentials are ready; a customer-owned overlay values file is prepared.

**Main Flow**:

1. Customer SRE creates Secrets for OIDC and any external-mode credentials.
2. SRE runs `INSIGHT_VALUES_FILES=deploy/values-base.yaml:deploy/values-prod.yaml:deploy/values-secrets.yaml ./deploy/scripts/install.sh`.
3. Installer pulls the chart from OCI (if `CHART_SOURCE=oci`), runs Helm, waits for pods.
4. Validator aborts the install if any required field is missing, naming the field.
5. On success, the UI is reachable through the configured ingress host.

**Postconditions**: umbrella, Airbyte and Argo are installed into the chosen namespace; OIDC-based auth is in force; production passwords live in customer Secrets.

**Alternative Flows**:

- **Skip Airbyte / skip Argo**: customer already operates these — set `SKIP_AIRBYTE=1` or `SKIP_ARGO=1`.

### 8.3 Constructor Platform tenant install

**ID**: `cpt-insightspec-usecase-dep-platform-tenant`

**Actors**: `cpt-insightspec-actor-platform-operator`

**Preconditions**: Constructor Platform provides a shared ClickHouse / MariaDB / Redpanda reachable from the tenant namespace; Secrets with credentials are already provisioned; namespace is empty.

**Main Flow**:

1. Operator pre-creates `insight-db-creds` in the target namespace with the platform-issued passwords, then prepares an overlay values file that sets `credentials.autoGenerate: false`, `clickhouse.deploy: false`, `mariadb.deploy: false`, `redis.deploy: false`, `redpanda.deploy: false`, each with the matching flat `host` / `port` / `passwordSecret` block.
2. Operator runs `SKIP_AIRBYTE=1` (platform provides Airbyte too) and the canonical installer for the umbrella only.
3. The umbrella's validator verifies every `<dep>.host` is present and every `<dep>.passwordSecret.{name,key}` resolves; `lookup` reads `insight-db-creds` and refuses to render with a missing or empty key.
4. `helm upgrade --install` deploys application services that talk to the shared platform infra through the platform ConfigMap.

**Postconditions**: tenant Insight install is live without bundled stateful infra; shared-platform services carry tenant data isolated at the database level (outside this subsystem's concern).

**Alternative Flows**:

- **Missing Secret**: validator aborts render with a message naming the missing Secret.

### 8.4 GitOps-managed install via ArgoCD

**ID**: `cpt-insightspec-usecase-dep-gitops`

**Actors**: `cpt-insightspec-actor-argocd-admin`

**Preconditions**: ArgoCD 2.6+ is installed; the cluster has cluster-admin-equivalent RBAC granted to ArgoCD; the target namespace is planned.

**Main Flow**:

1. Admin forks the repo and edits `deploy/gitops/*.yaml` to point at the fork URL and at the target namespace.
2. Admin commits and pushes.
3. Admin applies `deploy/gitops/root-app.yaml` once.
4. ArgoCD creates the four child Applications with sync-wave ordering; Airbyte + Argo + RBAC sync first, then Insight.
5. On subsequent changes, admin commits a new chart version in the Application manifest; ArgoCD reconciles.

**Postconditions**: the cluster matches Git; upgrades = merged PRs.

**Alternative Flows**:

- **No App-of-Apps**: admin applies the four Applications directly. Documented caveat: sync-wave ordering does not work across sibling Applications; admin must either wait manually between steps or accept initial retry loops.

### 8.5 Developer inner loop

**ID**: `cpt-insightspec-usecase-dep-dev-inner-loop`

**Actors**: `cpt-insightspec-actor-platform-developer`

**Preconditions**: developer has a checked-out repo, a working Kind cluster (or is about to create one via `dev-up.sh`), and is iterating on a backend service.

**Main Flow**:

1. Developer makes a code change in `src/backend/...`.
2. Developer runs `./dev-up.sh app` (or full `./dev-up.sh`), which rebuilds the affected image and loads it into Kind.
3. Helm upgrade runs; the pod is rolled.
4. Developer re-opens the Frontend port-forward and exercises the change.
5. When done, developer runs `./dev-down.sh` to tear everything down.

**Postconditions**: clean cluster state at the end of the session; no leftover resources.

## 9. Acceptance Criteria

- [ ] `helm template insight charts/insight` with the canonical `values.yaml` aborts with a readable message because OIDC and credentials are empty — zero successful renders of a misconfigured install.
- [ ] `helm template insight charts/insight -f deploy/values-dev.yaml` renders cleanly and produces every required Kubernetes object, including the three Argo `WorkflowTemplate` objects.
- [ ] `deploy/scripts/install.sh` installs the full stack on a fresh Kind cluster and all pods reach Ready without manual intervention, end-to-end in under 15 minutes on a typical developer laptop.
- [ ] Two concurrent installs in namespaces `insight-a` and `insight-b` on the same Kind cluster do not observe each other's Workflow objects.
- [ ] Applying `deploy/gitops/root-app.yaml` to a cluster with ArgoCD 2.6+ produces four child Applications that converge to Healthy in the Airbyte → Argo → Insight order enforced by sync-wave annotations.
- [ ] With `clickhouse.deploy: false` + a complete `clickhouse.host` / `.port` / `.passwordSecret` block pointing at a Constructor Platform instance, the resulting pods read from that external ClickHouse via the platform ConfigMap without modification to any subchart.
- [ ] `dev-up.sh` on Apple Silicon succeeds end-to-end without manual `docker pull --platform` calls; the `DEVLOG.md`-documented first-run failures do not regress.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Helm 3.14+ | Package manager for all three release paths (OCI pulls, multi-source). | p1 |
| Kubernetes 1.27+ | Target runtime; declared in the umbrella `kubeVersion`. | p1 |
| Airbyte chart 1.8.5 | Data extraction engine; installed as a separate Helm release. | p1 |
| Argo Workflows chart 0.45.x | Workflow engine for ingestion pipelines. | p1 |
| ArgoCD 2.6+ (GitOps path only) | Enterprise declarative deploy. | p2 |
| Kind 0.22+ (dev only) | Local Kubernetes for the developer inner loop. | p2 |
| Docker Desktop / containerd with `kind load` support (dev only) | Image ingestion into Kind. | p2 |
| OCI chart registry (`oci://ghcr.io/cyberfabric/charts/insight`) | Distribution target for the umbrella chart. | p2 |
| Bitnami Helm subcharts (MariaDB, Redis) via `bitnamilegacy` | Bundled-infra images still free after Bitnami's 2025 registry change. | p2 |
| Customer-managed OIDC issuer | Required for production installs (fail-fast validator enforces). | p1 |

## 11. Assumptions

- Customer clusters have a working default StorageClass and an ingress controller already installed; Deployment does not provision either.
- Customer SREs are comfortable with Helm values files, kubectl, and ArgoCD; the installers are not targeted at non-technical operators.
- The sibling repository `insight-front` (symlinked as `insight-front_symlink`) is present on developer machines for the dev wrapper's frontend build step.
- The bundled Airbyte and Argo Workflows versions remain viable for the next release cycle; upgrades to newer minors are handled in dedicated PRs with regression tests over ingestion workflows.
- On a shared cluster, tenant isolation is acceptable at the Kubernetes namespace boundary — workloads within a tenant namespace are mutually trusted.
- The Constructor Platform provides stable Secret resource references; tenants receive them out-of-band (not created by the installer).

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| No release automation yet — chart and images are hand-tagged. | A manual mis-tag ships an inconsistent umbrella. | Track as Open Item; write a CI pipeline that couples `git tag vX.Y.Z` → image build + chart package + OCI push; document the manual release process in the meantime. |
| Inline infra passwords previously had to be duplicated into app-service DSNs. | Drift between infra password and DSN produced a silently-broken install. | Resolved: `credentials.autoGenerate=true` writes `insight-db-creds` once and the umbrella derives all app-service Secrets (`insight-analytics-api-config`, `insight-identity-resolution-config`) from the same passwords. BYO mode reads the customer-supplied `insight-db-creds` instead. |
| Frontend image is `linux/amd64` only — Apple Silicon hosts rely on QEMU emulation or local rebuild. | Slow first pull and occasional emulation bugs on dev machines. | Dev wrapper builds the frontend from source as a workaround; infra team to publish multi-arch images. |
| Identity Resolution subchart ships as MVP stub that crashloops on empty bronze. | If operator flips `identityResolution.deploy: true` before any BambooHR sync, the release looks broken. | Keep default `identityResolution.deploy: false`; document the prerequisite in README; surface a clearer error message in the service itself (Backend concern). |
| Airbyte chart 1.9.x is deliberately skipped because its bundled app 2.0.x-alpha is not production-grade. | Customer asking for 1.9 gets a "no". | Document the policy in the Airbyte README; revisit when 2.0 GA ships. |
| Bitnami's late-2025 registry change means the MariaDB / Redis subcharts rely on `bitnamilegacy` + `global.security.allowInsecureImages`. | If Bitnami deprecates `bitnamilegacy`, both subcharts break. | Monitor Bitnami's policy; plan a migration to a vendored or self-hosted registry; enterprise customers are expected to use their own internal registry. |
