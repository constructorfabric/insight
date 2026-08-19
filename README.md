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
  - [Root scripts](#root-scripts)
  - [`src/`](#src)
  - [`docs/`](#docs)
  - [`inbox/`](#inbox)
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
│  Dashboards · Analytics · AI adoption · PR metrics · Team health │
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
│         Scheduling · Retry · State management · Monitoring       │
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
└── frontend/         ← SPA (React + TanStack source, Dockerfile, Helm)
```

### `docs/`

Product intent, shared API conventions, and the committed service contracts.

```
docs/
├── product/                      ← product vision and scenarios
├── shared/                       ← API guideline, status codes, versioning, glossary
│   ├── api-guideline/
│   └── glossary/
├── testing/                      ← test plan and reference orgs
├── domain/
│   └── identity-resolution/      ← cross-cutting: person registry, binding journal,
│                                    match rules, org hierarchy
└── components/backend/           ← backend service specs + committed OpenAPI contracts
    ├── analytics/                ← DESIGN, openapi.json
    ├── authenticator/            ← DESIGN, PRD, KEY-ROTATION, ADRs, openapi.json
    ├── gateway/                  ← DESIGN, ADR
    └── identity-resolution/      ← DESIGN, PRD, 14 ADRs, openapi.json
```

The `openapi.json` files are generated, not hand-written. CI regenerates each from
its service and fails the build on drift, so treat them as build output that happens
to be committed.

Design intent for everything else lives with the code it describes: the connector
`README.md` beside each `descriptor.yaml`, module docs under `src/backend/services/`,
and the pipeline conventions in `.cf-studio/config/rules/architecture.md`.

#### Backend specs — under review

The specs above are **retained but not yet trustworthy**. An audit against the
implementation found claims in each that the code contradicts. They are kept because
the surrounding material is worth correcting rather than discarding — but read them
against the code, not as authority, until the entries below are closed.

| Spec | Do not trust until checked |
|---|---|
| `analytics/DESIGN.md` | The endpoint table (§ Public API) lists routes the service does not register — the live surface is `/v1/metric-results`, `/v1/queries*`, `/v1/metric-drilldown*`, `/v1/metric-definitions`, `/v1/metrics*`. Metrics are addressed by `metric_key` (`table.column`), not a UUID. Server-side threshold evaluation and a `_thresholds` response field are described but not implemented. The RBAC column claims Tenant Admin on metric and threshold writes; the handlers declare only `.authenticated()`. The Redis alias cache and Redpanda invalidation are not wired. § 3.7 lists three tables that no runtime code reads. |
| `authenticator/DESIGN.md`, `PRD.md` | The admin revoke-by-user variant is documented on `DELETE /auth/sessions`; that route is public and self-only, and the admin variant is a separate authenticated route. CSRF is documented as covering logout only — it covers every state-changing `/auth/*` with a live session. A `tenants` claim and a `tenants` session field are described; both are a single `tenant_id`. The listed metric names are not the ones the crate registers. Several documented status codes are redirects in code. |
| `gateway/DESIGN.md` | The hygiene list describes `auth_request` / `auth_request_set` and `error_page`; routegen emits neither — it uses `access_by_lua_block` and shapes failures in Lua. The JWT cache key is the session cookie value, not a derived session id. |
| `components/backend/identity-resolution/.../DESIGN.md`, `PRD.md` | An internal `by-email` lookup is documented as present; it was removed. Ambiguous-profile is documented as `422` with structured members; the service returns `409`. `tenant_default_id` is described as a request-time fallback; no request path reads it. Subchart depth is documented as unbounded; it is capped and clamped. The subchart auth is documented as an `X-Insight-Person-Id` header; it is the gateway JWT. The PRD specifies FluentValidation and `IPersonsReader` — .NET idioms that do not exist in this Rust service. |
| `domain/identity-resolution/specs/DESIGN.md`, `PRD.md` | **Understates the code.** The whole operator resolution surface (`/v1/resolution/{bind,merge,detach,exclude}`, `/attention`, the account and person-accounts reads) is marked planned but is registered, handled and published in the contract. Contested-evidence handling, per-account bindings in divergent groups, author-aware conflict classification and per-account timestamp disambiguation are all described as gaps or future work and are all implemented. Tombstone handling is called inert; DELETE rows are kept deliberately. Going the other way: the v1 resolve macro is described as e-mail-only — it resolves through account bindings and returns nothing when claimants disagree — and the "writes only to its own tables" claim omits `org_chart`, `visibility`, `roles` and `person_roles`. |

The `openapi.json` next to each service is generated from the code and gated by CI,
so where a spec and the contract disagree, the contract is right.

### `inbox/`

Incoming documents pending triage and integration into `docs/`. Not yet canonical.

| Folder | Status |
|--------|--------|
| `architecture/` | Draft — architecture and permission notes |
| `stats/backend/`, `stats/frontend/` | Draft PRDs + ADRs |
| `IDENTITY_RESOLUTION.md` | Draft |

---

## Connector Coverage

| Domain | Sources | Silver Stream |
|--------|---------|---------------|
| Version Control | GitHub, GitHub Directory, GitLab, Bitbucket Cloud | `class_git_commits`, `class_git_pull_requests`, `class_git_repositories`, `class_git_file_changes` |
| Task Tracking | Jira | `class_task_projects`, `class_task_statuses`, `class_task_worklogs`, `class_task_field_history` |
| Collaboration | M365, Slack, Zoom, Zulip (proxy) | `class_collab_chat_activity`, `class_collab_email_activity`, `class_collab_meeting_activity`, `class_collab_document_activity` |
| Wiki | Confluence, Outline | `class_wiki_pages`, `class_wiki_activity`, `class_wiki_engagement` |
| Support | Zendesk | `class_support_activity` |
| AI Dev Tools | Cursor | `class_ai_dev_usage` |
| AI Tools | Claude Enterprise, Claude Team, ChatGPT Team | `class_ai_assistant_usage`, `class_ai_dev_usage`, `class_ai_overage` |
| HR / Directory | BambooHR, Active Directory, MS Entra | `class_people`, `class_person_attribute_claims`, `class_hr_events` |
| CRM | HubSpot | `class_crm_accounts`, `class_crm_contacts`, `class_crm_deals`, `class_crm_activities` |

---

## Key Concepts

**Identity Resolution** — Every Bronze table carries a source-native user identifier (`email`, `login`, `uuid`, etc.). The Identity Manager resolves these to a stable `person_id` in Silver Step 2, enabling cross-source analytics (e.g. joining a developer's git activity with their task tracker throughput and AI tool usage).

**Connector spec** — Each connector defines its Bronze table schemas, identity fields, Silver/Gold target streams, and open questions. The package `README.md` is the full technical spec; `descriptor.yaml` declares the image, schedule and required secret fields the toolkit reconciles.

**Extendability** — Adding a new data source means: (1) defining Bronze tables, (2) mapping identity fields, (3) routing to an existing or new Silver stream. The architecture is designed to accommodate new connectors without changes to existing pipelines.

---

## Quick Start

Two supported paths:

- **Local development (Docker Compose)** — [`./dev-compose.sh up`](./dev-compose.sh) runs the full stack on a developer laptop with only Docker. Default for day-to-day backend / frontend work.
- **Cluster deployment** — Cyberfabric engineers use the private `infra/insight-gitops` repository (Makefile-driven, OCI-pinned chart); the same path runs locally on a Kind/OrbStack cluster via `cd deploy/gitops && make deploy ENV=local` when you need Airbyte / Argo Workflows or the real cluster shape. External consumers of the umbrella Helm chart use it directly via `helm`, ArgoCD, Flux, or whatever orchestrator they already have; the chart contract lives at [`charts/insight/README.md`](charts/insight/README.md).

The two paths share a single first-run wizard, so the MariaDB / ClickHouse / tenant / dev-email answers are identical across them. The full guide for both is [CONTRIBUTING.md](CONTRIBUTING.md).

### Local development (Docker Compose)

For laptop development. No Rust / Node on the host — every build runs in a builder container; the only prerequisite is Docker (Engine 24+, compose v2).

```bash
git clone https://github.com/constructorfabric/insight.git
cd insight
./dev-compose.sh up        # first-run wizard, then builds + seeds the stack
```

The wizard prompts for local-vs-external MariaDB / ClickHouse, a dev-user email, and the frontend mode (defaults pull the published `insight-frontend` image). First `up` auto-seeds a demo dataset; open <http://localhost:3000>.

The compose stack does **not** ship Airbyte or Argo Workflows — for ingestion work use the Kubernetes path below. See [CONTRIBUTING.md](CONTRIBUTING.md) for the edit-build loop, frontend modes, seeding, and the `.env.compose` settings reference.

### Cluster deployment

Cyberfabric clusters are deployed from the private `infra/insight-gitops` repository — Makefile-driven, OCI-pinned umbrella chart, sealed secrets, L0/L2/L3 layered architecture. Engineers should refer to that repository's README; the deploy model is specified in [`deploy/gitops/README.md`](deploy/gitops/README.md).

External consumers run the umbrella chart directly:

```bash
helm install insight oci://ghcr.io/constructorfabric/charts/insight \
  --version <V> \
  --namespace insight --create-namespace \
  -f values.yaml
```

Chart versions are published per merge to `main`. The chart contract — values shape, integration modes, BYO credential keys, OIDC requirements — lives in [`charts/insight/README.md`](charts/insight/README.md).

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
| Identity Resolution | http://localhost:8086 | Rust |
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
| `insight-analytics` | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-analytics |
| `insight-identity-resolution` | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-identity-resolution |
| `insight-toolbox` | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-toolbox |
| `insight-seed` (test stands only) | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-seed |
| `insight-frontend` | this repo | https://github.com/constructorfabric/insight/pkgs/container/insight-frontend |
| `insight-jira-enrich` | **separate** `constructorfabric/insight-jira-enrich` | https://github.com/constructorfabric/insight-jira-enrich/pkgs/container/insight-jira-enrich |

> **Note**: jira-enrich lives in its own repo with an independent release cadence — a tag from this repo (e.g. `2026.04.28.10.34-b08b460`) does **not** exist for `insight-jira-enrich`. Pick the latest tag in that repo separately.

### CI/CD

GitHub Actions builds and pushes backend + toolbox container images on every merge to `main` (see [`.github/workflows/build-images.yml`](.github/workflows/build-images.yml)). Images are tagged `YYYY.MM.DD.HH.mm-<short-sha>` and `latest`. The same workflow publishes the umbrella Helm chart to `oci://ghcr.io/constructorfabric/charts/insight:<semver>` and auto-commits the version bumps back to `main`. See [`deploy/gitops/README.md`](deploy/gitops/README.md) for the gitops deploy contract.

To trigger manually: Actions → "Build & Push Container Images" → Run workflow.

### Running without Kubernetes

For fast iteration on individual components without K8s:

```bash
# Backend
cd src/backend
cargo run --bin insight-api-gateway -- run -c services/api-gateway/config/no-auth.yaml
# → http://localhost:8080/api/v1

# Frontend
cd src/frontend
pnpm install && pnpm dev
# → http://localhost:5173
```

See [`src/backend/services/LOCAL_DEV.md`](src/backend/services/LOCAL_DEV.md) for OIDC setup, MockOIDC, and other backend development options.

---

## Working with This Repo

- **Browse connectors** — Each package under `src/ingestion/connectors/{domain}/{source}/` carries its own `README.md`, `descriptor.yaml` and `dbt/` models.
- **Add a connector** — Follow the layout of any existing package, and the `/connector` skill.
- **Add source code** — Place code under `src/{component}/` — `src/ingestion/`, `src/backend/`, `src/frontend/`.
- **Constructor Studio** — Run `cfs` in a supported AI agent to activate assisted spec authoring, validation, and traceability. Constructor Studio is sourced from [github.com/constructorfabric/studio](https://github.com/constructorfabric/studio).
- **Inbox** — Documents in `inbox/` are drafts awaiting review. Do not reference them as canonical sources.

---

## Working with `docs/`

The `docs/` folder is the single source of truth for all product specifications, architectural decisions, and technical designs. Every document here is considered canonical and must go through a review process before being merged.

`docs/` carries product intent, shared API conventions and the committed service contracts. Component design lives beside the code it describes — update it in the same PR as the change.

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
