# Insight

> Decision Intelligence Platform

**Insight** is an extensible platform that collects operational data from across an organisation's toolchain, resolves all activity to unified person identities, and delivers actionable analytics for productivity improvement, bottleneck detection, process performance tracking, and team health reviews.

This repository is the **monorepo** for the Insight product. It contains:
- **`src/`** — source code for all platform components
- **`docs/`** — canonical product and technical specifications (specs, designs, ADRs)

<!-- toc -->

- [What Is Insight](#what-is-insight)
- [Architecture Overview](#architecture-overview)
  - [Components](#components)
  - [Bronze → Silver → Gold pipeline](#bronze--silver--gold-pipeline)
- [Repository Structure](#repository-structure)
  - [`src/`](#src)
  - [`docs/`](#docs)
  - [`inbox/`](#inbox)
  - [`cypilot/`](#cypilot)
- [Connector Coverage](#connector-coverage)
- [Key Concepts](#key-concepts)
- [Quick Start](#quick-start)
  - [Local development (Docker Compose)](#local-development-docker-compose)
  - [Cluster deployment](#cluster-deployment)
  - [Configure connectors](#configure-connectors)
  - [Services and ports](#services-and-ports)
  - [Image configuration](#image-configuration)
  - [CI/CD](#cicd)
  - [Running without Kubernetes](#running-without-kubernetes)
- [Working with This Repo](#working-with-this-repo)
- [Working with `docs/`](#working-with-docs)
  - [Document types](#document-types)
  - [Contribution workflow](#contribution-workflow)
  - [Summary](#summary)

<!-- /toc -->

---

## What Is Insight

Insight collects events and metrics from the tools teams already use — version control, task trackers, communication platforms, AI coding assistants, HR systems, and more — and unifies them into a single, identity-resolved data model.

**Primary use cases:**

| Use Case | Description |
|----------|-------------|
| **Process performance** | Cycle time, PR throughput, deployment frequency, task flow |
| **Productivity analytics** | Developer output, AI tool adoption, collaboration patterns |
| **Bottleneck detection** | Where work gets stuck across the delivery pipeline |
| **Team health** | Meeting load, async/sync balance, focus time |
| **Performance review** | Individual and team contribution signals across tools |
| **AI adoption tracking** | Usage, model distribution, and ROI across AI tools |

Insight is **not** a replacement for source systems — it reads from them, resolves identities, and provides a governed analytics layer on top.

---

## Architecture Overview

The solution consists of five main components:

```
┌──────────────────────────────────────────────────────────────────┐
│                          Frontend (SPA)                          │
│  Dashboards · Analytics · AI adoption · PR metrics · Team healt  │
└────────────────────────────┬─────────────────────────────────────┘
                             │ REST API (auth + data)
┌────────────────────────────▼─────────────────────────────────────┐
│                    Backend (REST API Server)                     │
│        Authentication · Authorization · User Management          │
│                     Data Proxy to Database                       │
└────────────────────────────┬─────────────────────────────────────┘
                             │ query
┌────────────────────────────▼─────────────────────────────────────┐
│                    Database (Analytics Store)                    │
│             Bronze → Silver → Gold (identity-resolved)           │
└────────────────────────────▲─────────────────────────────────────┘
                             │ write
┌────────────────────────────┴─────────────────────────────────────┐
│              Connector Orchestration Layer                       │
│         Scheduling · Retry · State management · Monitorin        │
└────────────────────────────▲─────────────────────────────────────┘
                             │ collect
┌────────────────────────────┴─────────────────────────────────────┐
│                         Connectors                               │
│   Git · Task Tracking · Collaboration · AI Tools · HR · CRM ...  │
└──────────────────────────────────────────────────────────────────┘
```

### Components

| # | Component | Description |
|---|-----------|-------------|
| 1 | **Connectors** | Source-specific integrations that pull raw data from external tools (git, task trackers, AI tools, HR systems, etc.) and write it to Bronze tables in the analytics database. |
| 2 | **Connector Orchestration** | Scheduling, retry, state management, and monitoring layer that coordinates connector runs and ensures reliable data ingestion. |
| 3 | **Database** | Analytics store holding the Bronze → Silver → Gold pipeline. Bronze is raw source data; Silver unifies schemas and resolves identities; Gold contains aggregated business metrics. |
| 4 | **Backend** | REST API server providing authentication, authorization, user management, and data proxy services. Serves as the central authentication gateway and data access layer, integrating with enterprise SSO systems. |
| 5 | **Frontend** | Single-page application (SPA) providing engineering managers, team leads, and developers with analytics and visualizations of git activity, AI tool adoption, pull request metrics, and team productivity. |

### Bronze → Silver → Gold pipeline

- **Bronze** — Raw, source-faithful tables. Field names and types preserved from the API. One table per entity type per source.
- **Silver Step 1** — Source tables unified into common schemas (e.g. `collab_chat_activity` merges Slack + Zulip + M365 Teams).
- **Silver Step 2** — Identity resolution: `email` / `login` / `user_id` resolved to canonical `person_id` via the Identity Manager.
- **Gold** — Aggregated, business-level metrics (cycle time, throughput, adoption rates, etc.).

---

## Repository Structure

### Root scripts

```
./dev-compose.sh      ← Docker Compose dev stack (default laptop path)
deploy/gitops/        ← Kubernetes path: `make deploy ENV=<env>`
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for both paths in full.

### `src/`

Source code for all platform components.

```
src/
├── ingestion/        ← Data pipeline (Airbyte + Argo + ClickHouse + dbt)
├── backend/          ← REST API server (Rust + cyberfabric-core)
└── frontend/         ← SPA deployment (Dockerfile + Helm; source in separate repo)
```

### `docs/`

Canonical product, domain, and component specifications. The single source of truth for everything architectural and technical.

```
docs/
├── components/                   ← per-component specifications
│   ├── connectors/               ← per-source connector specs (PRD + DESIGN + ADR)
│   │   ├── README.md             ← connector index + unified streams table
│   │   ├── git/                  ← GitHub, Bitbucket Server, GitLab
│   │   ├── task-tracking/        ← YouTrack, Jira
│   │   ├── collaboration/        ← Microsoft 365, Slack, Zoom, Zulip
│   │   ├── wiki/                 ← Confluence, Outline
│   │   ├── support/              ← Zendesk, Jira Service Management
│   │   ├── ai/                   ← Claude Admin, Claude Enterprise, Cursor, Windsurf, GitHub Copilot, JetBrains, OpenAI API, ChatGPT Team
│   │   ├── hr-directory/         ← BambooHR, Workday, LDAP / Active Directory
│   │   ├── crm/                  ← HubSpot, Salesforce
│   │   ├── ui-design/            ← Figma
│   │   └── testing/              ← Allure TestOps
│   │
│   ├── orchestrator/             ← connector orchestration layer specs
│   ├── backend/                  ← REST API server specs
│   └── frontend/                 ← SPA specs
│
├── domain/                       ← cross-cutting domain designs
│   ├── connector/                ← Connector Framework: generic architecture, automation
│   │   └── specs/DESIGN.md       │  boundary, BaseConnector, Unifier, onboarding
│   │                             │  (per-source details stay in components/connectors/)
│   └── identity-resolution/      ← Identity Resolution Service: person registry,
│       └── specs/DESIGN.md       │  alias store, Bootstrap Job, Golden Record,
│                                 │  match rules, org hierarchy, GDPR erasure
│
└── shared/                       ← shared API guidelines, status codes, versioning
    └── api-guideline/
```

**`docs/domain/` vs `docs/components/`:**

| Folder | Contains | When to look here |
|---|---|---|
| `docs/domain/` | Cross-cutting platform domains: concepts, algorithms, data models, and contracts that span multiple components | Understanding *how* identity resolution works, *what* the connector framework contract is, *why* the Medallion boundary is where it is |
| `docs/components/` | Per-component and per-connector specs: PRD (requirements), DESIGN (schemas, APIs, implementation details), ADR | Building, extending, or reviewing a specific connector, the backend, or the frontend |

### `inbox/`

Incoming documents pending triage and integration into `docs/`. Not yet canonical.

| Folder | Status | Intended destination |
|--------|--------|----------------------|
| `architecture/CONNECTORS_ARCHITECTURE.md` + `CONNECTOR_AUTOMATION.md` | **Synthesized** → `docs/domain/connector/specs/DESIGN.md` | Complete |
| `architecture/IDENTITY_RESOLUTION_V*.md` + `IDENTITY_RESOLUTION.md` | **Synthesized** → `docs/domain/identity-resolution/specs/DESIGN.md` | Complete |
| `architecture/PRODUCT_SPECIFICATION.md` | Draft | `docs/domain/` or root product spec |
| `architecture/permissions/` | Draft ADRs | `docs/components/backend/specs/ADR/` |
| `airbyte-declarative-standalone/` | **Migrated** | `docs/components/connectors/collaboration/m365/` |
| `stats/backend/` | Draft ADRs | `docs/components/backend/specs/ADR/` |
| `stats/frontend/` | Draft | `docs/components/frontend/specs/` |
| `streams/` | Draft schemas | `docs/components/connectors/` — per-source stream definitions |

### `cypilot/`

This repo uses [Cypilot](https://github.com/cyberfabric/cyber-pilot) — an AI agent framework for spec authoring, validation, and traceability. The `cypilot/` directory contains the project-specific configuration (artifact registry, rules, kit bindings). The engine itself lives in the upstream repo.

---

## Connector Coverage

| Domain | Sources | Silver Stream |
|--------|---------|---------------|
| Version Control | GitHub, Bitbucket Server, GitLab | `class_commits`, `class_pr_activity` |
| Task Tracking | YouTrack, Jira | `class_task_tracker` |
| Collaboration | M365, Slack, Zoom, Zulip | `class_communication_metrics`, `class_document_metrics` |
| Wiki | Confluence, Outline | `class_wiki_pages`, `class_wiki_activity` |
| Support | Zendesk, JSM | `class_support_activity` |
| AI Dev Tools | Cursor, Windsurf, Copilot, JetBrains | `class_ai_dev_usage` |
| AI Tools | Claude Admin, Claude Enterprise, OpenAI API, ChatGPT Team | `class_ai_api_usage`, `class_ai_tool_usage` |
| HR / Directory | BambooHR, Workday, LDAP | `class_people`, `class_org_units` |
| CRM | HubSpot, Salesforce | TBD |
| Design Tools | Figma | `class_design_activity` |
| Quality / Testing | Allure TestOps | TBD |

---

## Key Concepts

**Identity Resolution** — Every Bronze table carries a source-native user identifier (`email`, `login`, `uuid`, etc.). The Identity Manager resolves these to a stable `person_id` in Silver Step 2, enabling cross-source analytics (e.g. joining a developer's git activity with their task tracker throughput and AI tool usage).

**Connector spec** — Each connector defines its Bronze table schemas, identity fields, Silver/Gold target streams, and open questions. The `{source}.md` file is the full technical spec; `specs/PRD.md` captures the code-agnostic requirements.

**Extendability** — Adding a new data source means: (1) defining Bronze tables, (2) mapping identity fields, (3) routing to an existing or new Silver stream. The architecture is designed to accommodate new connectors without changes to existing pipelines.

---

## Quick Start

Two supported paths:

- **Local development (Docker Compose)** — [`./dev-compose.sh up`](./dev-compose.sh) runs the full stack on a developer laptop with only Docker. Default for day-to-day backend / frontend work.
- **Cluster deployment** — Cyberfabric engineers use the private `infra/insight-gitops` repository (Makefile-driven, OCI-pinned chart); the same path runs locally on a Kind/OrbStack cluster via `cd deploy/gitops && make deploy ENV=local` when you need Airbyte / Argo Workflows or the real cluster shape. External consumers of the umbrella Helm chart use it directly via `helm`, ArgoCD, Flux, or whatever orchestrator they already have; the chart contract lives at [`charts/insight/README.md`](charts/insight/README.md).

The two paths share a single first-run wizard, so the MariaDB / ClickHouse / tenant / dev-email answers are identical across them. The full guide for both is [CONTRIBUTING.md](CONTRIBUTING.md).

### Local development (Docker Compose)

For laptop development. No Rust / .NET / Node on the host — every build runs in a builder container; the only prerequisite is Docker (Engine 24+, compose v2).

```bash
git clone https://github.com/constructorfabric/insight.git
cd insight
./dev-compose.sh up        # first-run wizard, then builds + seeds the stack
```

The wizard prompts for local-vs-external MariaDB / ClickHouse, a dev-user email, and the frontend mode (defaults pull the published `insight-front` image). First `up` auto-seeds a demo dataset; open <http://localhost:3000>.

The compose stack does **not** ship Airbyte or Argo Workflows — for ingestion work use the Kubernetes path below. See [CONTRIBUTING.md](CONTRIBUTING.md) for the edit-build loop, frontend modes, seeding, and the `.env.compose` settings reference.

### Cluster deployment

Cyberfabric clusters are deployed from the private `infra/insight-gitops` repository — Makefile-driven, OCI-pinned umbrella chart, sealed secrets, L0/L2/L3 layered architecture. Engineers should refer to that repository's README; the deploy model is specified in [`docs/components/deployment/gitops/README.md`](docs/components/deployment/gitops/README.md).

External consumers run the umbrella chart directly:

```bash
helm install insight oci://ghcr.io/constructorfabric/charts/insight \
  --version <V> \
  --namespace insight --create-namespace \
  -f values.yaml
```

Chart versions are published per merge to `main`; see [ADR-0001](docs/components/deployment/specs/ADR/0001-chart-publishing-on-merge.md) for the publish-on-merge contract. The chart contract — values shape, integration modes, BYO credential keys, OIDC requirements — lives in [`charts/insight/README.md`](charts/insight/README.md).

### Configure connectors

Once the umbrella is running:

```bash
export KUBECONFIG=/path/to/cluster.kubeconfig

# 1. Apply per-source K8s Secrets (one file per connector you want active)
kubectl -n insight apply -f src/ingestion/secrets/connectors/m365.yaml
kubectl -n insight apply -f src/ingestion/secrets/connectors/bamboohr.yaml

# 2. Tenant config — defaults to discovering all Secrets labeled
#    `app.kubernetes.io/part-of=insight` in the namespace.
cp src/ingestion/connections/example-tenant.yaml.example \
   src/ingestion/connections/default.yaml

# 3. Port-forward Airbyte API (the toolkit calls it on localhost:8001)
kubectl -n insight port-forward svc/airbyte-airbyte-server-svc 8001:8001 &

# 4. Register connector definitions in Airbyte
./src/ingestion/airbyte-toolkit/register.sh collaboration/m365
./src/ingestion/airbyte-toolkit/register.sh hr-directory/bamboohr

# 5. Create sources, destinations, connections, bronze databases
./src/ingestion/update-connections.sh default

# 6. One-shot sync per connector
./src/ingestion/run-sync.sh m365 default
./src/ingestion/run-sync.sh bamboohr default

# 7. Watch the workflow
kubectl -n insight get workflows -l tenant=default --watch
```

### Services and ports

For local Docker Compose development every web service publishes a host port (override any `*_PORT` in `.env.compose` on conflict):

| Service | URL | Notes |
|---|---|---|
| Frontend | http://localhost:3000 | SPA |
| API Gateway | http://localhost:8080 | `/api/v1`; auth disabled in the `no-auth` config |
| Analytics API | http://localhost:8081 | |
| Identity | http://localhost:8082 | .NET 9 |
| ClickHouse HTTP | http://localhost:8123 | `/play` for browser SQL |
| MariaDB | localhost:3306 | |
| Redis | localhost:6379 | |

The compose stack does not run Airbyte or Argo Workflows — those live on the Kubernetes path. For cluster deployments services are reached via the configured ingress hostname (or `kubectl port-forward`).

### Image configuration

The chart fails fast if any image tag is empty — there are **no `:latest` defaults** anywhere.

For local Docker Compose development each backend service is built locally by default; skip the build for any service and pull its published image instead by setting `<SVC>_IMAGE` in `.env.compose` (e.g. `API_GATEWAY_IMAGE=ghcr.io/constructorfabric/insight-api-gateway:latest`) or with `./dev-compose.sh up --from-ghcr=<svc>`. See [CONTRIBUTING.md](CONTRIBUTING.md) for the build targets and frontend modes.

For cluster deployments image tags flow through automatically: the umbrella chart's CI bumps each subchart's `appVersion` on every merge to `main`, and the subchart templates default `image.tag` to `.Chart.AppVersion`. Env overlays only need to pin a tag explicitly for a hotfix scenario (testing one service at a different tag than the one bundled in the umbrella version). Image source repos:

| Image | Source repo | Tags |
|---|---|---|
| `insight-api-gateway` | `constructorfabric/insight` (this repo) | https://github.com/constructorfabric/insight/pkgs/container/insight-api-gateway |
| `insight-analytics-api` | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-analytics-api |
| `insight-identity` | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-identity |
| `insight-toolbox` | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-toolbox |
| `insight-front` | **separate** `constructorfabric/insight-front` | https://github.com/constructorfabric/insight/pkgs/container/insight-front |
| `insight-jira-enrich` | **separate** `constructorfabric/insight-jira-enrich` | https://github.com/constructorfabric/insight/pkgs/container/insight-jira-enrich |

> **Note**: frontend and jira-enrich live in their own repos with independent release cadences — a backend tag (e.g. `2026.04.28.10.34-b08b460`) does **not** exist for `insight-front`. Pick the latest tag in the frontend's repo separately.

### CI/CD

GitHub Actions builds and pushes backend + toolbox container images on every merge to `main` (see [`.github/workflows/build-images.yml`](.github/workflows/build-images.yml)). Images are tagged `YYYY.MM.DD.HH.mm-<short-sha>` and `latest`. The same workflow publishes the umbrella Helm chart to `oci://ghcr.io/constructorfabric/charts/insight:<semver>` and auto-commits the version bumps back to `main`. See [ADR-0001](docs/components/deployment/specs/ADR/0001-chart-publishing-on-merge.md) for the publish-on-merge rationale and [`docs/components/deployment/gitops/README.md`](docs/components/deployment/gitops/README.md) for the gitops deploy contract.

To trigger manually: Actions → "Build & Push Container Images" → Run workflow.

### Running without Kubernetes

For fast iteration on individual components without K8s:

```bash
# Backend
cd src/backend
cargo run --bin insight-api-gateway -- run -c services/api-gateway/config/no-auth.yaml
# → http://localhost:8080/api/v1

# Frontend
cd ../insight-front       # or via the insight-front_symlink at repo root
npm install && npm run dev
# → http://localhost:5173
```

See [`src/backend/services/LOCAL_DEV.md`](src/backend/services/LOCAL_DEV.md) for OIDC setup, MockOIDC, and other backend development options.

---

## Working with This Repo

- **Browse specs** — Start at [`docs/components/connectors/README.md`](docs/components/connectors/README.md) for the connector index, or [`docs/domain/`](docs/domain/) for cross-cutting platform concepts (identity resolution, connector framework).
- **Understand a domain** — Read the relevant `docs/domain/{domain}/specs/DESIGN.md` first. These documents describe the platform's core algorithms, data contracts, and architectural decisions that span multiple components.
- **Add a connector** — Follow the layout in any existing `docs/components/connectors/{domain}/{source}/` directory. Use `specs/PRD.md` for requirements and `specs/DESIGN.md` for table schemas and pipeline mappings.
- **Add source code** — Place code under `src/{component}/`. The structure mirrors `docs/components/` — `src/connectors/`, `src/backend/`, `src/frontend/`, `src/orchestrator/`.
- **Cypilot** — Run `cypilot on` in a supported AI agent to activate assisted spec authoring, validation, and traceability. Cypilot is sourced from [github.com/cyberfabric/cyber-pilot](https://github.com/cyberfabric/cyber-pilot).
- **Inbox** — Documents in `inbox/` are drafts awaiting review. Do not reference them as canonical sources.

---

## Working with `docs/`

The `docs/` folder is the single source of truth for all product specifications, architectural decisions, and technical designs. Every document here is considered canonical and must go through a review process before being merged.

`docs/` describes the **architecture and intent** of the platform. The corresponding implementation lives in `src/`. When adding or changing source code, the relevant spec in `docs/components/{component}/specs/DESIGN.md` should be updated in the same PR (or a follow-up ADR opened if it's a significant design change).

### Document types

Each component or connector has a `specs/` subdirectory with three document types:

| File | Purpose | Who writes it |
|------|---------|---------------|
| `specs/PRD.md` | Business and product requirements — actors, use cases, functional requirements, NFRs. **Code-agnostic**: no schemas, no implementation details. | Product / domain owners |
| `specs/DESIGN.md` | Technical design — Bronze table schemas, identity resolution mechanics, Silver/Gold pipeline mappings, data flow. | Engineering |
| `specs/ADR/` | Architecture Decision Records — individual decisions that affect the design. | Engineering |

### Contribution workflow

#### Adding or updating requirements (PRD)

Business requirements, use cases, actor definitions, and functional/non-functional requirements belong in `specs/PRD.md` of the relevant component or connector.

1. Create a branch.
2. Edit `specs/PRD.md` — add or update requirements. Keep it code-agnostic: describe **what** the system must do, not how.
3. Open a PR for review. Once approved, merge.

#### Updating the technical design (DESIGN)

`specs/DESIGN.md` is the authoritative technical specification for a component. It must reflect the current agreed-upon design at all times.

**Minor changes** (style fixes, formatting, clarifications, small field additions) can be committed directly to `specs/DESIGN.md` via a standard PR.

**Major changes** (data schema changes, new pipeline stages, significant architectural decisions, breaking changes to existing models) require an ADR first:

1. Create a new ADR in `specs/ADR/` describing the proposed change (context, options considered, decision, consequences).
2. Open a PR with the ADR only.
3. Once the ADR is approved and merged, update `specs/DESIGN.md` in a follow-up commit or PR to reflect the accepted decision.

This ensures every significant design change has a traceable decision record before the canonical design document is updated.

#### ADR naming convention

```
specs/ADR/NNN-short-description.md
```

Example: `specs/ADR/001-use-email-as-identity-key.md`

### Summary

```
Propose requirement change       →  edit PRD.md       →  PR  →  merge
Propose minor design change      →  edit DESIGN.md    →  PR  →  merge
Propose major design change      →  new ADR           →  PR  →  merge  →  update DESIGN.md
```
