# DESIGN — YouTrack Connector

> Version 1.0 — May 2026
> Scope: Bronze layer (DECOMPOSITION §2.1–§2.4). Silver/enrich layer (§2.5–§2.10) is future scope — see `README.md` in this folder.
> Based on: [PRD.md](./PRD.md), [DECOMPOSITION.md](./DECOMPOSITION.md)

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
- [4. Additional context](#4-additional-context)
  - [Datetime handling](#datetime-handling)
  - [Activity categories](#activity-categories)
  - [Identity stamping](#identity-stamping)
  - [Page size config keys](#page-size-config-keys)
  - [Future scope reference](#future-scope-reference)
- [5. Traceability](#5-traceability)
  - [DECOMPOSITION feature coverage](#decomposition-feature-coverage)
- [6. Non-Applicability Statements](#6-non-applicability-statements)

<!-- /toc -->

---

## 1. Architecture Overview

### 1.1 Architectural Vision

The YouTrack connector is an Airbyte declarative (nocode) YAML manifest that extracts ten streams from the JetBrains YouTrack REST API and writes them to per-source Bronze tables in ClickHouse. The declarative approach is consistent with all other declarative connectors in the project (m365, bamboohr, zoom, claude-admin) and uses the project-wide `airbyte/source-declarative-manifest` runtime — no custom Python CDK.

Five streams are directories (full-refresh / overwrite): `youtrack_projects`, `youtrack_user`, `youtrack_agiles`, `youtrack_sprints`, `youtrack_issue_link_types`. One stream is incremental on the `updated` cursor: `youtrack_issue`. Three streams are substreams of `youtrack_issue` with `incremental_dependency: true`: `youtrack_issue_history`, `youtrack_comments`, `youtrack_worklogs`. One stream is a substream of `youtrack_projects`: `youtrack_project_custom_fields`.

Bronze tables follow the project-wide convention: `engine=ReplacingMergeTree(_version)`, `order_by=[unique_key]`, identity columns `tenant_id` + `source_id` + `_version` injected via `AddFields` in the manifest. The `unique_key` column carries the row's natural identity (e.g. `youtrack_id` for issues, composite `(issue_id, activity_id)` for activity history).

Phase 1 targets both YouTrack Cloud and YouTrack Server with a single manifest — the REST shapes for the ten endpoints used are identical across deployments (verified in Phase 1 research). Server-only endpoints (Hub admin, Helpdesk) are out of scope.

The Builder-UI compatibility constraint drives a structural choice that distinguishes this connector from `task-tracking/jira`: all `requester`, `paginator`, `add_fields`, and substream-parent definitions are **inlined per stream**, not extracted to a `definitions:` block with whole-object `$ref`. The Jira connector pre-dates this constraint and is flagged as an anti-template for new manifests.

### 1.2 Architecture Drivers

**PRD Reference**: [PRD.md](./PRD.md)

**ADRs backing this design**:

- `cpt-insightspec-adr-youtrack-project-scoped-fields` — [ADR-001 project-scoped custom fields](./ADR/ADR-001-project-scoped-custom-fields.md)
- `cpt-insightspec-adr-youtrack-activitiespage-cursor` — [ADR-002 activitiesPage cursor pagination](./ADR/ADR-002-activitiespage-cursor-pagination.md)
- `cpt-insightspec-adr-youtrack-no-whitelist` — [ADR-003 no-whitelist full-ingestion](./ADR/ADR-003-no-whitelist-full-ingestion.md)

#### Functional Drivers

| Requirement | Design Response |
|---|---|
| `cpt-insightspec-fr-youtrack-bronze-scaffold` | Declarative source package at `src/ingestion/connectors/task-tracking/youtrack/` with `connector.yaml`, `descriptor.yaml`, `dbt/schema.yml`, `README.md`, `youtrack.yaml.example`. See `cpt-insightspec-component-youtrack-airbyte-manifest`. |
| `cpt-insightspec-fr-youtrack-bronze-auth-bearer` | `BearerAuthenticator { api_token: "{{ config['youtrack_token'] }}" }` inlined per stream's `requester`. See `cpt-insightspec-component-youtrack-airbyte-manifest`. |
| `cpt-insightspec-fr-youtrack-bronze-retry-policy` | `DefaultErrorHandler` per stream — `Retry-After` honoured, 429/503 retried with backoff, 401/403 fail-fast, 404 on substream URL → drop partition. See `cpt-insightspec-component-youtrack-airbyte-manifest`. |
| `cpt-insightspec-fr-youtrack-identity-stamping` | `AddFields[{type: AddedFieldDefinition, path: [tenant_id], value: "{{ config['insight_tenant_id'] }}"}, …]` on every stream. See `cpt-insightspec-component-youtrack-airbyte-manifest`. |
| `cpt-insightspec-fr-youtrack-stream-projects` | `DeclarativeStream youtrack_projects` — see `cpt-insightspec-component-youtrack-stream-projects`. |
| `cpt-insightspec-fr-youtrack-stream-users` | `DeclarativeStream youtrack_user` — see `cpt-insightspec-component-youtrack-stream-users`. |
| `cpt-insightspec-fr-youtrack-stream-agiles-sprints` | Two streams: `DeclarativeStream youtrack_agiles` (root, full-refresh) + `DeclarativeStream youtrack_sprints` (substream of `youtrack_agiles`). See `cpt-insightspec-component-youtrack-stream-agiles-sprints`. |
| `cpt-insightspec-fr-youtrack-stream-link-types` | `DeclarativeStream youtrack_issue_link_types` — see `cpt-insightspec-component-youtrack-stream-link-types`. |
| `cpt-insightspec-fr-youtrack-stream-issue-incremental` | `DeclarativeStream youtrack_issue` with `DatetimeBasedCursor` on `updated` (cursor formats: `%ms`, `%Y-%m-%dT%H:%M:%S`). See `cpt-insightspec-component-youtrack-stream-issue`. |
| `cpt-insightspec-fr-youtrack-stream-activities-cursor` | `DeclarativeStream youtrack_issue_history` with `CursorPagination` (`afterCursor` / `hasAfter`), `categories` whitelist. See `cpt-insightspec-component-youtrack-stream-issue-history`. |
| `cpt-insightspec-fr-youtrack-substream-routing-youtrack-id` | All issue substreams route via `stream_partition.youtrack_id` (parent `record['id']`). See `cpt-insightspec-component-youtrack-stream-issue-history`, `cpt-insightspec-component-youtrack-stream-comments`, `cpt-insightspec-component-youtrack-stream-worklogs`. |
| `cpt-insightspec-fr-youtrack-stream-comments` | `DeclarativeStream youtrack_comments` (substream of `youtrack_issue`). See `cpt-insightspec-component-youtrack-stream-comments`. |
| `cpt-insightspec-fr-youtrack-stream-worklogs` | `DeclarativeStream youtrack_worklogs` (substream of `youtrack_issue`). See `cpt-insightspec-component-youtrack-stream-worklogs`. |
| `cpt-insightspec-fr-youtrack-stream-issue-links` | No separate Bronze stream — `youtrack_issue.links[]` kept as JSON. Flattening deferred to Silver staging (DECOMPOSITION §2.5). See `cpt-insightspec-component-youtrack-stream-issue-links` (deferred). |
| `cpt-insightspec-fr-youtrack-incremental-dependency` | `parent_stream_configs[…].incremental_dependency: true` on every issue substream. See `cpt-insightspec-component-youtrack-stream-issue-history` (and siblings). |
| `cpt-insightspec-fr-youtrack-stream-project-custom-fields` | `DeclarativeStream youtrack_project_custom_fields` (substream of `youtrack_projects`). See `cpt-insightspec-component-youtrack-stream-project-custom-fields`. |
| `cpt-insightspec-fr-youtrack-custom-field-bundles` | `fields` query parameter includes `bundle(values(...))` inlining. See `cpt-insightspec-component-youtrack-stream-project-custom-fields`. |
| `cpt-insightspec-fr-youtrack-identity-by-email` | `youtrack_user` stream captures `email`, `login`, `id` — Silver resolution chain enforced by future scope §2.5. See `cpt-insightspec-component-youtrack-stream-users`. |

#### NFR Allocation

| NFR ID | Allocated To | Verification |
|---|---|---|
| `cpt-insightspec-nfr-youtrack-secret-rotation` | Airbyte source runtime reads config at sync-start (project-wide property — no connector-specific code) | Manual test: rotate Secret, wait for next sync, confirm new token in use |
| `cpt-insightspec-nfr-youtrack-no-log-token` | `airbyte/source-declarative-manifest` standard secret redaction; no custom logging in manifest | Inspection of `read` output after deliberate-failure sync |
| `cpt-insightspec-nfr-youtrack-directory-overwrite` | `sync_mode: full_refresh` + `destination_sync_mode: overwrite` on five directory streams | `source.sh discover` emits expected sync modes |
| `cpt-insightspec-nfr-youtrack-issue-replacingmergetree` | `engine=ReplacingMergeTree(_version)` + `order_by=[unique_key]` in `dbt/schema.yml` source-block table config | `check-dbt-conventions` skill — automated audit |
| `cpt-insightspec-nfr-youtrack-idempotency` | Source-side: deterministic `unique_key` per row. Storage-side: `ReplacingMergeTree` dedup | E2E smoke run from DECOMPOSITION §2.10 (future) |
| `cpt-insightspec-nfr-youtrack-schema-drift-detection` | `dbt parse` CI gate (PR #382) + `generate-schema.sh youtrack` | Pre-commit + CI |

### 1.3 Architecture Layers

```text
┌────────────────────────────────────────────────────────────────┐
│ Operator                                                       │
│  ├─ K8s Secret (insight-youtrack-<source-id>)                  │
│  └─ Argo CronWorkflow / run-sync.sh                            │
└──────────────────────────────┬─────────────────────────────────┘
                               │
                  ┌────────────▼────────────┐
                  │ Airbyte Source Runtime  │
                  │  (source-declarative-   │
                  │   manifest)             │
                  └────────────┬────────────┘
                               │
                  ┌────────────▼─────────────────────────────────┐
                  │  connector.yaml (DeclarativeSource)          │
                  │   - 10 streams (5 root, 5 substream)         │
                  │   - inlined requester / paginator / auth     │
                  │   - AddFields identity-stamp                 │
                  └────────────┬─────────────────────────────────┘
                               │  HTTP
                  ┌────────────▼────────────┐
                  │ YouTrack REST API       │
                  └────────────┬────────────┘
                               │  Airbyte Protocol JSON
                  ┌────────────▼────────────┐
                  │ ClickHouse Destination  │
                  │  bronze_youtrack.*      │
                  │  ReplacingMergeTree     │
                  └────────────┬────────────┘
                               │
                  ┌────────────▼────────────────────────────────┐
                  │ dbt source declarations (dbt/schema.yml)    │
                  │  source: bronze_youtrack                    │
                  │  tables: 10 streams                         │
                  │  ↓                                          │
                  │  (Future scope §2.5: per-source staging     │
                  │   tagged silver:class_task_*)               │
                  └─────────────────────────────────────────────┘
```

## 2. Principles & Constraints

### 2.1 Design Principles

#### Declarative-First

- [ ] `p1` - **ID**: `cpt-insightspec-principle-youtrack-declarative-first`

The connector ships as a single `connector.yaml` declarative manifest with no custom Python CDK code. All semantics (auth, pagination, cursor, identity stamping, error handling) are expressed in the YAML. The runtime is `airbyte/source-declarative-manifest`.

**Rationale**: Declarative manifests open in the Airbyte Builder UI, enable deterministic CI validation via `validate-strict`, and avoid maintenance of bespoke Python code.

#### Symmetry With Jira

- [ ] `p1` - **ID**: `cpt-insightspec-principle-youtrack-symmetry-with-jira`

Every YouTrack Bronze concept maps 1:1 to a Jira Bronze concept where the semantics align. Where YouTrack REST semantics differ (activitiesPage cursor, project-scoped custom fields, no project whitelist), a dedicated ADR documents the divergence.

**Rationale**: Down-stream Silver/Gold layers are source-agnostic (`union_by_tag('silver:class_task_*')`). Symmetric Bronze shapes minimize per-source staging logic and reuse existing dbt invariants.

#### Identity by Email With Login Fallback

- [ ] `p1` - **ID**: `cpt-insightspec-principle-youtrack-identity-by-email`

The Bronze `youtrack_user` table captures `email`, `login`, and YouTrack-internal `id` for every user. Silver identity resolution (future scope §2.5) prefers `email` → falls back to `login` → falls back to `id`.

**Rationale**: YouTrack Hub allows email suppression. The fallback chain ensures identity continuity at the cost of one resolution step.

#### Cursor Pagination for Activities

- [ ] `p1` - **ID**: `cpt-insightspec-principle-youtrack-cursor-for-activities`

`youtrack_issue_history` paginates via `CursorPagination(afterCursor, hasAfter)`, not via offset. All other streams use offset (`$skip` / `$top`).

**Rationale**: YouTrack does not support stable offset pagination for `activitiesPage`. Cursor pagination is the only correct shape; mixing with offset on the same stream would produce duplicates and gaps.

#### Project-Scoped Registry

- [ ] `p1` - **ID**: `cpt-insightspec-principle-youtrack-project-scoped-registry`

Custom-field definitions are discovered per project via `/api/admin/projects/{id}/customFields`, not globally. The Bronze `youtrack_project_custom_fields` table is keyed on `(project_id, field_id)`.

**Rationale**: YouTrack's data model permits the same logical field to have different IDs and bundle values across projects. A global registry would either over-shadow project-specific configurations or require post-hoc disambiguation.

#### Silver Ownership Boundary

- [ ] `p1` - **ID**: `cpt-insightspec-principle-youtrack-silver-ownership-boundary`

This connector emits Bronze tables only. All transformations into the `class_task_*` Silver union are owned by the future per-source dbt staging (DECOMPOSITION §2.5). The connector's `descriptor.yaml` sets `dbt_select: ""` to prevent accidental dbt invocation against models that do not yet exist.

**Rationale**: A clear ownership boundary between Bronze ingestion and Silver staging prevents one PR from accidentally shipping incomplete Silver work.

### 2.2 Constraints

#### No-Whitelist Full-Ingestion Scope

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-youtrack-no-whitelist`

The connector ingests every project the permanent token can reach. There is no `youtrack_project_short_names` K8s Secret field or per-project allowlist in the manifest.

**Rationale**: Token-scoped access is sufficient — operators control the ingestion surface by limiting token permissions. Adding a manifest-level allowlist would duplicate the permission boundary and introduce drift.

**ADR**: Connector ADR-003 — No-whitelist full-ingestion scope.

#### K8s-Secret Identity

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-youtrack-k8s-secret-identity`

All connector identity comes from a single K8s Secret. The fields are: `insight_tenant_id`, `insight_source_id`, `youtrack_base_url`, `youtrack_token`, and three optional config keys (`youtrack_start_date`, `youtrack_page_size`, `youtrack_activities_page_size`). No identity is hard-coded in the manifest or in the descriptor.

#### activitiesPage Cursor Pagination

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-youtrack-activitiespage-cursor`

`youtrack_issue_history` pagination **MUST** use `CursorPagination` against `response.afterCursor` and `response.hasAfter`. Offset pagination is rejected by YouTrack on this endpoint.

**ADR**: Connector ADR-002 — activitiesPage cursor pagination.

#### Project-Scoped Custom Fields

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-youtrack-project-scoped-fields`

`youtrack_project_custom_fields` **MUST** be a substream of `youtrack_projects`. A single instance-wide `/api/customFieldSettings/customFields` request does not return project-scoped IDs and bundle values.

**ADR**: Connector ADR-001 — Project-scoped custom fields ingestion.

#### Builder-UI Compatibility

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-youtrack-builder-ui-compat`

The manifest **MUST** pass `source.sh validate-strict` (Airbyte Builder UI's JSON-schema validation with no `$ref` resolution). This forbids whole-object `$ref`, requires `type: AddedFieldDefinition` on every `AddFields.fields[]` item, requires the schema URL `http://json-schema.org/schema#`, requires the type-array order `[<type>, "null"]`, and requires integer-typed slots to be either literal integers or Jinja templates only on slots where the schema accepts a templated string (`page_size` does; `concurrency_level.default_concurrency` does not).

## 3. Technical Architecture

### 3.1 Domain Model

```text
┌────────────────────┐         ┌─────────────────────┐
│ Project            │1       *│ ProjectCustomField  │
│ (youtrack_projects)│─────────│ (youtrack_project_  │
│                    │         │  custom_fields)     │
│ - id (youtrack_id) │         │ - id                │
│ - shortName        │         │ - field.id          │
│ - name             │         │ - bundle.values[]   │
│ - archived         │         │ - canBeEmpty        │
└─────────┬──────────┘         │ - isPublic          │
          │                    └─────────────────────┘
          │1
          │
          │*                       ┌─────────────────────┐
┌─────────▼──────────┐ 1        * │ ActivityItem        │
│ Issue              │────────────│ (youtrack_issue_    │
│ (youtrack_issue)   │             │  history)           │
│                    │             │ - id                │
│ - id (youtrack_id) │             │ - timestamp         │
│ - idReadable       │             │ - $type             │
│ - summary          │             │ - category          │
│ - updated (ms)     │             │ - field             │
│ - links[] (JSON)   │             │ - added[] / removed[]│
│ - customFields[]   │             └─────────────────────┘
└─────────┬──────────┘
          │1
          ├────────────────────┐
          │*                   │*
┌─────────▼──────────┐ ┌──────▼───────────┐
│ Comment            │ │ WorkItem         │
│ (youtrack_comments)│ │ (youtrack_       │
│ - id, text, author │ │  worklogs)       │
│ - created, updated │ │ - id, duration   │
│ - deleted          │ │ - author, date   │
└────────────────────┘ └──────────────────┘

┌────────────────────┐         ┌─────────────────────┐
│ AgileBoard         │1       *│ Sprint              │
│ (youtrack_agiles)  │─────────│ (youtrack_sprints)  │
│ - id, name         │         │ - id, name          │
│ - projects[]       │         │ - start, finish     │
└────────────────────┘         │ - archived, goal    │
                               └─────────────────────┘

┌────────────────────┐         ┌─────────────────────┐
│ User               │         │ IssueLinkType       │
│ (youtrack_user)    │         │ (youtrack_issue_    │
│ - id, login, email │         │  link_types)        │
│ - fullName         │         │ - id, name          │
│ - banned, guest    │         │ - sourceToTarget    │
└────────────────────┘         │ - targetToSource    │
                               │ - directed          │
                               └─────────────────────┘
```

### 3.2 Component Model

#### Airbyte Manifest

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-airbyte-manifest`

##### Why this component exists

Single source of truth for the connector's runtime behaviour — auth, ten stream definitions, pagination strategies, cursor, error handler, identity-stamp transformations, spec, and metadata. Loaded by `airbyte/source-declarative-manifest` at sync start.

##### Responsibility scope

`version`, `type: DeclarativeSource`, `check.stream_names = ["youtrack_projects"]`, the `definitions` block (only narrow leaf-level entries that satisfy Builder-UI rules), all ten `streams[]`, `spec.connection_specification` (required and optional config keys), `concurrency_level`, `metadata`.

##### Responsibility boundaries

Does NOT contain custom Python code. Does NOT contain dbt logic. Does NOT contain Argo workflow definitions. Per-stream details (paginator, cursor, transformations) belong to each stream's component below.

##### Related components (by ID)

- `cpt-insightspec-component-youtrack-descriptor` — descriptor that ties the source definition to dbt + schedule + namespace.
- `cpt-insightspec-component-youtrack-dbt-source-decl` — dbt source declarations consumed by future Silver staging.
- Every per-stream component (`cpt-insightspec-component-youtrack-stream-*`).

**File**: `src/ingestion/connectors/task-tracking/youtrack/connector.yaml`

#### Descriptor

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-descriptor`

**Why this component exists**: Insight-internal descriptor (loaded by `connect.sh`) that ties the Airbyte source definition to its dbt downstream, sync schedule, and ClickHouse namespace.

**Responsibility scope**: `name: youtrack`, `version`, `schedule: "0 3 * * *"`, `workflow: sync`, `connection.namespace: bronze_youtrack`, `dbt_select: ""` (no-op until §2.5 lands), `secret.required_fields`, `secret.optional_fields`.

**File**: `src/ingestion/connectors/task-tracking/youtrack/descriptor.yaml`

#### dbt Source Declarations

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-dbt-source-decl`

**Why this component exists**: Registers the ten Bronze tables in the dbt graph so future Silver staging models can `source('bronze_youtrack', 'youtrack_issue')` etc.

**Responsibility scope**: `sources: - name: bronze_youtrack { tables: [...] }`. Each table block carries the `unique_key`, `tests`, and `meta` (engine, order_by) needed by `check-dbt-conventions`.

**File**: `src/ingestion/connectors/task-tracking/youtrack/dbt/schema.yml`

#### Stream — `youtrack_projects`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-projects`

**Why this component exists**: Project directory; substream parent for `youtrack_project_custom_fields`; identity anchor for cross-project analytics.

**Responsibility scope**: `GET /api/admin/projects` with `fields=id,shortName,name,description,archived`, offset pagination via `$skip`/`$top` (page_size from `config.get('youtrack_page_size', 100)`), `sync_mode: full_refresh`, `destination_sync_mode: overwrite`, `AddFields` for identity, `InlineSchemaLoader` with the regenerated JSON Schema.

**Responsibility boundaries**: Does NOT include archived projects' custom-field drift inferred from history (Silver layer concern).

#### Stream — `youtrack_user`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-users`

**Why this component exists**: User directory; anchor for identity resolution.

**Responsibility scope**: `GET /api/users?fields=id,login,fullName,email,banned,guest,avatarUrl`, offset pagination, full-refresh/overwrite. Note: `email` may be `null` for Hub-privacy-suppressed users.

#### Stream — `youtrack_agiles` + `youtrack_sprints`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-agiles-sprints`

**Why this component exists**: Sprint metadata required for velocity / carry-over analytics. YouTrack exposes sprints only nested under boards — substream is the only viable shape.

**Responsibility scope**:

- `youtrack_agiles`: `GET /api/agiles?fields=id,name,projects(id,shortName)`, offset pagination, full-refresh/overwrite.
- `youtrack_sprints`: `GET /api/agiles/{youtrack_id}/sprints?fields=id,name,start,finish,archived,goal`, substream of `youtrack_agiles` keyed on `agile_id`.

#### Stream — `youtrack_issue_link_types`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-link-types`

**Why this component exists**: Issue-link metadata (direction labels, aggregation rules) required to interpret per-issue link rows during Silver staging.

**Responsibility scope**: `GET /api/issueLinkTypes?fields=id,name,sourceToTarget,targetToSource,directed,aggregation,readOnly`, offset pagination, full-refresh/overwrite.

#### Stream — `youtrack_issue`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-issue`

**Why this component exists**: The pivot — every issue substream depends on this stream via `incremental_dependency: true`. The `updated` cursor drives the entire issue-scope sync.

**Responsibility scope**: `GET /api/issues?query=updated:{from}..{to} order by: updated asc&fields={ISSUE_FIELDS}`, offset pagination (page_size from `config.get('youtrack_page_size', 100)`), `DatetimeBasedCursor` on `updated` with `cursor_datetime_formats: [%ms, %Y-%m-%dT%H:%M:%S]`, `datetime_format: %Y-%m-%dT%H:%M:%S`, `step: P30D`, `cursor_granularity: PT1S`, `lookback_window: PT1H`.

**Responsibility boundaries**: Does NOT decode custom-field values into rows (Silver layer concern via `youtrack_issue.customFields[]` JSON column).

#### Stream — `youtrack_issue_history`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-issue-history`

**Why this component exists**: Activity history is the source of truth for state change replay (future scope). Bronze captures it 1:1 with no transformation.

**Responsibility scope**: `GET /api/issues/{youtrack_id}/activitiesPage?fields={ACTIVITIES_FIELDS}&$top=200&categories={ACTIVITY_CATEGORIES}&reverse=true`, `CursorPagination` (`cursor_value: "{{ response.afterCursor }}"`, `stop_condition: "{{ response.hasAfter is false }}"`), page_size from `config.get('youtrack_activities_page_size', 200)`, substream of `youtrack_issue` keyed on `parent.record['id']` exposed as `stream_partition.youtrack_id`, `incremental_dependency: true`.

**Responsibility boundaries**: Does NOT replay activities into per-field state (Silver layer concern, future §2.6).

#### Stream — `youtrack_comments`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-comments`

**Why this component exists**: Comment volume per person is a collaboration signal.

**Responsibility scope**: `GET /api/issues/{youtrack_id}/comments?fields=id,text,textPreview,created,updated,author(id,login,fullName,email),deleted,visibility(...)`, offset pagination, substream of `youtrack_issue` keyed on `youtrack_id`, `incremental_dependency: true`.

#### Stream — `youtrack_worklogs`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-worklogs`

**Why this component exists**: Effort measurement per person per issue.

**Responsibility scope**: `GET /api/issues/{youtrack_id}/timeTracking/workItems?fields=id,duration(minutes),date,author(...),text,worktype(id,name)`, offset pagination, substream of `youtrack_issue` keyed on `youtrack_id`, `incremental_dependency: true`.

#### Stream — `youtrack_issue_links` (deferred)

- [ ] `p2` - **ID**: `cpt-insightspec-component-youtrack-stream-issue-links`

**Why this component exists (future)**: Issue dependency / duplicate / blocker graph required for analytics.

**Responsibility scope (future)**: Projection of `youtrack_issue.links[]` (JSON column already captured in `youtrack_issue`) into a flat `(source_issue, target_issue, link_type_id, direction)` table. Materialised at Silver staging (DECOMPOSITION §2.5) — no Bronze stream emitted in this PR.

#### Stream — `youtrack_project_custom_fields`

- [ ] `p1` - **ID**: `cpt-insightspec-component-youtrack-stream-project-custom-fields`

**Why this component exists**: Project-scoped custom-field registry feeds `class_task_field_metadata` (future scope).

**Responsibility scope**: `GET /api/admin/projects/{youtrack_id}/customFields?fields=id,field(id,name,localizedName,fieldType(id,valueType,isMultiValue)),bundle(id,values(id,name,description,archived,color(id,presentation),ordinal)),canBeEmpty,ordinal,emptyFieldText,isPublic`, offset pagination, substream of `youtrack_projects` keyed on `project.id` (renamed to `project_id` in emitted rows).

### 3.3 API Contracts

| YouTrack endpoint | Stream | Sync mode | Pagination |
|---|---|---|---|
| `GET /api/admin/projects` | `youtrack_projects` | `full_refresh` | Offset (`$skip`/`$top`) |
| `GET /api/users` | `youtrack_user` | `full_refresh` | Offset |
| `GET /api/agiles` | `youtrack_agiles` | `full_refresh` | Offset |
| `GET /api/agiles/{id}/sprints` | `youtrack_sprints` | `full_refresh` (substream) | Offset |
| `GET /api/issueLinkTypes` | `youtrack_issue_link_types` | `full_refresh` | Offset |
| `GET /api/issues?query=updated:…` | `youtrack_issue` | `incremental` (cursor: `updated`) | Offset |
| `GET /api/issues/{id}/activitiesPage` | `youtrack_issue_history` | substream + `incremental_dependency` | Cursor (`afterCursor`/`hasAfter`) |
| `GET /api/issues/{id}/comments` | `youtrack_comments` | substream + `incremental_dependency` | Offset |
| `GET /api/issues/{id}/timeTracking/workItems` | `youtrack_worklogs` | substream + `incremental_dependency` | Offset |
| `GET /api/admin/projects/{id}/customFields` | `youtrack_project_custom_fields` | `full_refresh` (substream) | Offset |

### 3.4 Internal Dependencies

- `src/ingestion/tools/declarative-connector/` — `source.sh validate-strict`, `validate`, `check`, `discover`, `read`. The connector cannot be edited safely without this toolkit.
- `src/ingestion/silver/task-tracking/schema.yml` — Silver union contract. Future per-source staging models (DECOMPOSITION §2.5) must respect column names and types declared here.
- `.cf-studio/config/rules/code-conventions.md` — no-inline-Python rule applies to any future Python helper.
- `docs/domain/ingestion-data-flow/specs/` — Bronze engine + `unique_key` conventions.

### 3.5 External Dependencies

- **YouTrack REST API** — `https://<tenant>.youtrack.cloud/api/` (Cloud) or `https://<host>/youtrack/api/` (Server). Auth via Bearer permanent token.
- **`airbyte/source-declarative-manifest`** — pinned via `descriptor.yaml.version`. Runtime that loads `connector.yaml`.
- **ClickHouse** — destination (project-wide convention `engine=ReplacingMergeTree(_version)`).

### 3.6 Interactions & Sequences

#### Sequence — Connector `check`

- [ ] `p1` - **ID**: `cpt-insightspec-seq-youtrack-connector-check`

```text
Operator                connect.sh           Airbyte runtime          YouTrack API
   │                       │                       │                      │
   │ source.sh check ──────►                       │                      │
   │                       │ docker run check ────►                       │
   │                       │                       │ GET /api/admin/      │
   │                       │                       │ projects?$top=1 ────►│
   │                       │                       │◄── 200 OK            │
   │                       │ ◄── {"status":         │                      │
   │                       │      "SUCCEEDED"}      │                      │
   │ ◄─── ✓ exit 0         │                       │                      │
```

#### Sequence — Secret Discovery

- [ ] `p1` - **ID**: `cpt-insightspec-seq-youtrack-secret-discovery`

```text
Operator             K8s                    Airbyte runtime
   │                   │                          │
   │ kubectl apply ────►                          │
   │                   │ Secret stored            │
   │                   │                          │
   │                   │       sync start         │
   │                   │ ◄────────────────────────│
   │                   │ Secret read into ENV ────►
   │                   │                          │ config.youtrack_token = <value>
```

#### Sequence — Directory Refresh

- [ ] `p1` - **ID**: `cpt-insightspec-seq-youtrack-directory-refresh`

```text
Airbyte                                         YouTrack API
   │                                                │
   │ GET /api/admin/projects?$skip=0&$top=100 ─────►│
   │ ◄─── [...100 projects]                         │
   │ GET /api/admin/projects?$skip=100&$top=100 ───►│
   │ ◄─── [...remaining]                            │
   │ destinationSyncMode: overwrite                 │
   │ → bronze_youtrack.youtrack_projects rewritten  │
```

#### Sequence — Incremental Issue Sync

- [ ] `p1` - **ID**: `cpt-insightspec-seq-youtrack-issue-incremental`

```text
Airbyte                                         YouTrack API
   │ Load state: { updated: 2026-04-01T00:00:00 }   │
   │ GET /api/issues?query=updated:{state}..{now}   │
   │     order by: updated asc                      │
   │ &fields={ISSUE_FIELDS}&$skip=0&$top=100 ──────►│
   │ ◄─── [...100 issues, max(updated)=T+1]         │
   │ → emit records, advance cursor to T+1          │
   │ ◄─── next page                                 │
   │ STATE: { updated: <last-issue-updated> }       │
```

#### Sequence — Activities Cursor Walk

- [ ] `p1` - **ID**: `cpt-insightspec-seq-youtrack-activities-cursor-walk`

```text
Airbyte (substream)                             YouTrack API
   │ partition: youtrack_id=2-12345                 │
   │ GET /api/issues/2-12345/activitiesPage?$top=   │
   │     200&categories=...&reverse=true ──────────►│
   │ ◄─── { activities: [...], afterCursor: "abc",  │
   │        hasAfter: true }                        │
   │ GET ?afterCursor=abc&...&hasAfter=true ───────►│
   │ ◄─── { activities: [...], hasAfter: false }    │
   │ STOP                                           │
```

#### Sequence — Substream Dependency

- [ ] `p1` - **ID**: `cpt-insightspec-seq-youtrack-substream-dependency`

```text
youtrack_issue (parent)                youtrack_{history,comments,worklogs}
   │                                                  │
   │ emit issue { id=2-12345, updated=T+1 } ──────────►
   │                                       fire 3 substream partitions per issue
   │                                                  │
   │ incremental_dependency: true                     │
   │ → only issues with updated > state.cursor        │
   │   re-trigger their substream calls               │
```

#### Sequence — Custom-Field Discovery

- [ ] `p1` - **ID**: `cpt-insightspec-seq-youtrack-custom-field-discovery`

```text
Airbyte (substream)                             YouTrack API
   │ For each project from youtrack_projects:        │
   │   partition: youtrack_id=PROJ-1                 │
   │   GET /api/admin/projects/PROJ-1/customFields   │
   │       ?fields=...,bundle(values(...))──────────►│
   │ ◄─── [...field defs incl. bundle values]        │
   │   inject project_id=PROJ-1                      │
   │   emit records                                  │
```

### 3.7 Database schemas & tables

#### Bronze Namespace

- [ ] `p1` - **ID**: `cpt-insightspec-db-youtrack-bronze-namespace`

ClickHouse schema: `bronze_youtrack`. Registered as a dbt source block in `src/ingestion/connectors/task-tracking/youtrack/dbt/schema.yml`. Engine: `ReplacingMergeTree(_version)`. Order by: `[unique_key]`. Identity columns on every table: `tenant_id`, `source_id`, `unique_key`, `_version`.

#### `bronze_youtrack.youtrack_projects`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_projects`

| Column | Type | Notes |
|---|---|---|
| `tenant_id` | `String` | from K8s Secret |
| `source_id` | `String` | from K8s Secret |
| `unique_key` | `String` | = `youtrack_id` |
| `youtrack_id` | `String` | YouTrack internal `id` (e.g. `0-1`) |
| `short_name` | `String` | e.g. `PROJ` |
| `name` | `String` | human-readable name |
| `description` | `String?` | nullable |
| `archived` | `Bool` | |
| `_version` | `UInt64` | from Airbyte `_airbyte_emitted_at` |

#### `bronze_youtrack.youtrack_user`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_user`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `unique_key`, `_version` | (identity) | |
| `youtrack_id` | `String` | YouTrack internal `id` |
| `login` | `String` | YouTrack login |
| `full_name` | `String?` | |
| `email` | `String?` | nullable when Hub privacy on |
| `banned` | `Bool` | |
| `guest` | `Bool` | |
| `avatar_url` | `String?` | |

#### `bronze_youtrack.youtrack_agiles`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_agiles`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `unique_key`, `_version` | (identity) | |
| `youtrack_id` | `String` | board id |
| `name` | `String` | |
| `projects_json` | `String` | embedded `projects[]` array as JSON |

#### `bronze_youtrack.youtrack_sprints`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_sprints`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `_version` | (identity) | |
| `unique_key` | `String` | = `agile_id ‖ '|' ‖ sprint_id` |
| `agile_id` | `String` | parent board id |
| `youtrack_id` | `String` | sprint id |
| `name` | `String` | |
| `start` | `DateTime64(3)?` | |
| `finish` | `DateTime64(3)?` | |
| `archived` | `Bool` | |
| `goal` | `String?` | |

#### `bronze_youtrack.youtrack_issue_link_types`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_issue_link_types`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `unique_key`, `_version` | (identity) | |
| `youtrack_id` | `String` | link type id |
| `name` | `String` | |
| `source_to_target` | `String?` | |
| `target_to_source` | `String?` | |
| `directed` | `Bool` | |
| `aggregation` | `Bool` | |
| `read_only` | `Bool` | |

#### `bronze_youtrack.youtrack_issue`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_issue`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `unique_key`, `_version` | (identity) | |
| `youtrack_id` | `String` | internal id (`record['id']`) |
| `id_readable` | `String?` | human-readable (`record.get('idReadable')`); may be null |
| `project_id` | `String` | |
| `summary` | `String?` | |
| `description` | `String?` | |
| `reporter_json` | `String?` | reporter as JSON |
| `created` | `DateTime64(3)` | from ms |
| `updated` | `DateTime64(3)` | **cursor field**, from ms via `%ms` |
| `resolved` | `DateTime64(3)?` | |
| `custom_fields_json` | `String` | embedded `customFields[]` |
| `links_json` | `String` | embedded `links[]` |
| `comments_json` | `String?` | optional embedded comments (also full substream) |

#### `bronze_youtrack.youtrack_issue_history`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_issue_history`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `_version` | (identity) | |
| `unique_key` | `String` | = `issue_youtrack_id ‖ '|' ‖ activity_id` |
| `youtrack_id` | `String` | parent issue's internal id (NOT activity id) |
| `activity_id` | `String` | activity row id |
| `timestamp` | `DateTime64(3)` | activity timestamp, from ms |
| `activity_type` | `String` | `$type` discriminator |
| `category_id` | `String` | activity category id |
| `field_id` | `String?` | when applicable |
| `field_name` | `String?` | when applicable |
| `author_id` | `String?` | |
| `added_json` | `String?` | `added[]` array |
| `removed_json` | `String?` | `removed[]` array |
| `target_json` | `String?` | activity target object |

#### `bronze_youtrack.youtrack_comments`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_comments`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `_version` | (identity) | |
| `unique_key` | `String` | = `comment_id` |
| `comment_id` | `String` | |
| `youtrack_id` | `String` | parent issue's internal id |
| `text` | `String?` | |
| `text_preview` | `String?` | |
| `created` | `DateTime64(3)` | from ms |
| `updated` | `DateTime64(3)?` | from ms |
| `author_id` | `String?` | |
| `deleted` | `Bool` | |
| `visibility_json` | `String?` | |

#### `bronze_youtrack.youtrack_worklogs`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_worklogs`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `_version` | (identity) | |
| `unique_key` | `String` | = `worklog_id` |
| `worklog_id` | `String` | |
| `youtrack_id` | `String` | parent issue's internal id |
| `duration_minutes` | `Int64` | |
| `date` | `DateTime64(3)` | from ms |
| `author_id` | `String?` | |
| `text` | `String?` | |
| `worktype_id` | `String?` | |
| `worktype_name` | `String?` | |

#### `bronze_youtrack.youtrack_issue_links` (deferred — no Bronze table)

- [ ] `p2` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_issue_links` *(deferred)*

`youtrack_issue_links` rows are kept inside `youtrack_issue.links_json` and projected at Silver staging (future scope §2.5). No separate Bronze table is created in this PR. The ID is reserved here so the future per-source staging projection has a target to reference.

#### `bronze_youtrack.youtrack_project_custom_fields`

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-youtrack-youtrack_project_custom_fields`

| Column | Type | Notes |
|---|---|---|
| `tenant_id`, `source_id`, `_version` | (identity) | |
| `unique_key` | `String` | = `project_id ‖ '|' ‖ youtrack_id` |
| `project_id` | `String` | parent project's id |
| `youtrack_id` | `String` | project-scoped custom-field id |
| `field_id` | `String?` | underlying global field id (when present) |
| `field_name` | `String` | |
| `field_localized_name` | `String?` | |
| `field_type_id` | `String?` | e.g. `enum`, `version`, `period` |
| `field_value_type` | `String?` | |
| `is_multi_value` | `Bool` | |
| `can_be_empty` | `Bool` | |
| `is_public` | `Bool` | |
| `ordinal` | `Int32?` | |
| `empty_field_text` | `String?` | |
| `bundle_id` | `String?` | |
| `bundle_values_json` | `String?` | `bundle.values[]` |

## 4. Additional context

### Datetime handling

- `youtrack_issue.updated` is epoch ms (integer). The cursor uses `%ms` for record-value parsing and `%Y-%m-%dT%H:%M:%S` for state-value parsing on resume.
- YouTrack `query` parameter accepts ISO 8601 with T-separator and no braces: `updated: 2026-01-01T00:00:00 .. 2026-04-23T00:00:00`. Braces around datetimes (legacy v1 behaviour) are rejected by current YouTrack Cloud with `invalid_query`.
- `DatetimeBasedCursor.cursor_granularity: PT1S` is mandatory whenever `step` is set (CDK enforces this — runtime error without it).

### Activity categories

The 23 activity categories the connector requests in `youtrack_issue_history` (whitelist trims chatter like attachment churn):

`CustomFieldCategory, SummaryCategory, DescriptionCategory, CommentsCategory, AttachmentsCategory, IssueCreatedCategory, IssueResolvedCategory, LinksCategory, TagsCategory, ProjectCategory, SprintCategory, VotersCategory, VisibilityCategory, TimeTrackingCategory, WorkItemTypeCategory, WorkItemAuthorCategory, WorkItemDescriptionCategory, WorkItemDurationCategory, WorkItemUsesMarkdownCategory, WorkItemCategory, IssueTagsCategory, IssueWatcherCategory, PermittedGroupCategory`.

The exact list is encoded in `connector.yaml` per the v1/v2 donor research; the canonical reference is Connector ADR-002.

### Identity stamping

Every record carries `tenant_id` and `source_id` set in the manifest via `AddFields[{type: AddedFieldDefinition, path: [tenant_id], value: "{{ config['insight_tenant_id'] }}"}, …]`. The `type: AddedFieldDefinition` field is mandatory for Builder-UI compatibility (a missing `type` is a `validate-strict` error).

### Page size config keys

Three K8s Secret optional fields drive pagination:

| Key | Default | Used by |
|---|---|---|
| `youtrack_page_size` | `100` | Every offset paginator (issues, directories, users, comments, worklogs, custom fields) |
| `youtrack_activities_page_size` | `200` | `youtrack_issue_history` cursor paginator |
| `youtrack_start_date` | `'2020-01-01'` | `youtrack_issue` `DatetimeBasedCursor.start_datetime` |

All three are wired via `"{{ config.get('youtrack_page_size', 100) }}"` (Jinja-interpolated string, accepted by `OffsetIncrement`/`CursorPagination.page_size`).

### Future scope reference

See `README.md` in this folder for the complete future-scope plan covering features 2.5–2.10 (dbt staging, Rust enrich, Argo workflow, silver plug-in, E2E tests).

## 5. Traceability

### DECOMPOSITION feature coverage

| DECOMPOSITION feature | Covered by |
|---|---|
| §2.1 Bronze Airbyte Manifest Skeleton | `cpt-insightspec-component-youtrack-airbyte-manifest`, `cpt-insightspec-component-youtrack-descriptor`, `cpt-insightspec-component-youtrack-dbt-source-decl` |
| §2.2 Bronze Directory Streams | `cpt-insightspec-component-youtrack-stream-projects`, `cpt-insightspec-component-youtrack-stream-users`, `cpt-insightspec-component-youtrack-stream-agiles-sprints`, `cpt-insightspec-component-youtrack-stream-link-types` |
| §2.3 Bronze Incremental Issues & Substreams | `cpt-insightspec-component-youtrack-stream-issue`, `cpt-insightspec-component-youtrack-stream-issue-history`, `cpt-insightspec-component-youtrack-stream-comments`, `cpt-insightspec-component-youtrack-stream-worklogs`, `cpt-insightspec-component-youtrack-stream-issue-links` (deferred) |
| §2.4 Project-Scoped Custom Field Ingestion | `cpt-insightspec-component-youtrack-stream-project-custom-fields` |
| §2.5–§2.10 (future scope) | See `README.md` in this folder |

## 6. Non-Applicability Statements

The following PRD-level concerns are intentionally NOT addressed in this DESIGN:

- **Silver/Gold transformations** — boundary owned by the future per-source staging models (DECOMPOSITION §2.5–§2.7). This DESIGN's responsibility ends at `bronze_youtrack.*` table emission.
- **Identity resolution to canonical `person_id`** — Silver layer responsibility (identity manager domain).
- **Cross-source dedup** — Silver layer responsibility.
- **Real-time / event-driven sync** — out of scope; this connector is daily-batch.
- **Per-project allowlisting** — explicitly rejected (Connector ADR-003).
- **Custom-field auto-promotion to silver columns** — Silver layer responsibility.
