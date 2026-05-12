# PRD — Metric Catalog

> For a plain-English summary aimed at product, compliance, and onboarding audiences, see [`PRD_human_readable.md`](./PRD_human_readable.md). The document you are reading is the canonical requirements spec; the companion is advisory.

<!-- toc -->

- [Changelog](#changelog)
- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
  - [3.1 Module-Specific Environment Constraints](#31-module-specific-environment-constraints)
- [4. Scope](#4-scope)
  - [4.1 In Scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Catalog Storage](#51-catalog-storage)
  - [5.2 Threshold Storage and Resolution](#52-threshold-storage-and-resolution)
  - [5.3 Read API](#53-read-api)
  - [5.4 Admin Write API](#54-admin-write-api)
  - [5.5 Seed and Migration](#55-seed-and-migration)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
  - [UC-001 Admin Tunes a Threshold](#uc-001-admin-tunes-a-threshold)
  - [UC-002 Product Team Adds a New Metric](#uc-002-product-team-adds-a-new-metric)
  - [UC-003 Consumer Hydrates the Catalog](#uc-003-consumer-hydrates-the-catalog)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Open Questions](#13-open-questions)
- [14. Current-State Gap Analysis (Backend)](#14-current-state-gap-analysis-backend)
  - [Present](#present)
  - [Missing — v1](#missing--v1)
  - [Missing — Follow-on](#missing--follow-on)
  - [Relevant Open Pull Requests](#relevant-open-pull-requests)
  - [Implementation Sequencing (informative)](#implementation-sequencing-informative)

<!-- /toc -->

## Changelog

- **v1.10** (current): Reframed tenant-custom metrics from "out of scope, cross-layer PRD required" to "additive follow-on, v1 schema pre-wired". **`metric_catalog` gains a nullable `tenant_id` column in v1** with a CHECK constraint forcing `tenant_id IS NULL` — identical pattern to `lock_expires_at`. All v1 rows remain global (product-owned); the column ships dead, ready for the follow-on FR that lifts the CHECK and adds CRUD for tenant-custom metrics. This chooses path (a) from the v1.9 §13 OQ (nullable `tenant_id` on the existing table) over path (b) (separate `metric_catalog_tenant_custom` table). Rationale: architectural symmetry with thresholds — product-owned global baseline + tenant-owned additive overlay — without the footgun of making product metadata tenant-editable. Cross-tenant comparability preserved because product-owned metrics stay product-owned (`cpt-metric-cat-fr-metadata-writes` unchanged); tenant customization becomes additive (append a row with `tenant_id = X`), never a replacement. **§4.2 bullet on tenant-custom rewritten** from "warrants its own PRD" to "ships in a follow-on PRD; v1 schema ready". **§13 OQ on tenant-custom closed**: path (a) chosen. **Three new §13 OQs opened for the follow-on scope**: (i) query-layer storage model for tenant-custom — tenant-scoped rows in `analytics.metrics` vs a formula DSL poverh existing metrics (prototype demonstrated both UX shapes) vs both; (ii) disable-for-me slot — additive table letting a tenant hide a product-owned metric for themselves without affecting others; (iii) PK evolution strategy — keep v1 `PK = metric_key` and rebuild when tenant-custom lands, or ship v1 with synthetic `id` + `UNIQUE(tenant_id, metric_key)` to avoid PK rebuild. **New §11 Assumption** and **§12 Risk** on tenant maintenance burden: when tenant-custom lands, product commits silver backward-compat only for metrics in the global catalog; tenants owning custom metrics accept responsibility for updating them when depended-upon silver columns change. This is the contract that makes silver evolution tractable — append-only-forever obligation on product-owned silver coverage only, not on every field a tenant might have bound to. Read endpoint response shape is unchanged in v1; when the follow-on lifts the CHECK, resolution filters rows by `tenant_id IN (caller, NULL)` and optionally surfaces `owner: 'product' | 'tenant'` as an additive field.
- **v1.9**: Disambiguated two previously-conflated metadata restrictions: (a) *editing* existing metric metadata per-tenant (labels, units, formats) — covered by the global `metric_catalog` keying choice in v1.7 and by `cpt-metric-cat-fr-metadata-writes`; (b) *adding* tenant-specific custom metrics that exist only for one tenant — not addressed by v1.7 and genuinely out of v1 scope. Custom per-tenant metrics require tenant-specific `query_ref` rows in `analytics.metrics`, tenant-aware catalog rows, and potentially tenant-specific silver/gold transforms — a cross-layer design reaching well beyond the catalog. Added explicit Out-of-Scope bullet in §4.2, new Open Question in §13 with two additive extension paths (nullable `tenant_id` column vs dedicated `metric_catalog_tenant_custom` table), and a parallel note in `PRD_human_readable.md`. No schema or API change in v1 — the global `metric_catalog` shape keeps both extension paths available as additive follow-ons. Also added a pointer from PRD.md to the new human-readable companion document.
- **v1.8**: Audit-tightening, Glossary cleanup, and DESIGN-handoff hardening. (1) `lock_reason` is now NOT NULL whenever `is_locked = true` (enforced via DB CHECK `is_locked = false OR lock_reason IS NOT NULL`). Empty `lock_reason` in an audit event would be a red flag for SOC2 / ISO auditors; requiring it at write-time keeps the audit trail useful. (2) New `p2` follow-on FR `cpt-metric-cat-fr-integrity-check` — periodic background job flagging `metric_threshold` rows whose `role_slug` / `team_id` no longer matches any live taxonomy. Gated on `role_catalog` delivery; v1 accepts the v1-only dangling-reference window explicitly in §12. (3) Extended the closed OQ on product-default locks with emergency-migration mitigation: migration-only override policy stands, but pipelines for compliance-emergency migrations may skip non-essential gates to compress hours-to-days down to ~1 hour while preserving git audit trail. Runtime feature-flag bypass explicitly rejected. (4) **Glossary cleanup**: rewrote `Metric threshold`, `Threshold scope`, `Role-scoped override` to match the v1.4 scope model (they were still v1.3 `dashboard`-based); added `Role`, `Team`, `Dashboard scope (rejected)` entries. (5) Added implementation note to `cpt-metric-cat-fr-scoped-thresholds`: `GET /catalog/metrics` must resolve via one bulk fetch plus in-memory walk; N+1 fanout fails the read-latency NFR. (6) **Constraint enforcement requires both DB CHECK and app-layer validation** (not either-or): DB is ultimate backstop against writes bypassing the API; app layer owns user-facing errors and covers old-MariaDB / SeaORM-bug edge cases. (7) **Closed OQ on audit destination** — ship dedicated `threshold_lock_audit` MariaDB table in v1 alongside structured logs. Table gives queryable long-term retention (≥ 1 year default) independent of log-aggregation retention policy, unblocks future CSV / JSON export from admin UI, and decouples audit retention from log-cost economics. Full lifecycle (`lock_set` / `lock_cleared` / `bypass_attempt`) lands in the same table. (8) Added risk and mitigation for Auth service delivery slipping and blocking the catalog write path — decouple via a Rust trait + test-double stub for staging / local-dev; production gates on the real Auth implementation.
- **v1.7**: Five Enterprise-grade hardening changes. (1) Promoted cross-replica cache invalidation from Open Question to a `p1` NFR (`cpt-metric-cat-nfr-cross-replica-invalidation`); DESIGN must pick a mechanism (shared cache, pub-sub broadcast, or equivalent) that makes admin-write visibility observable across all analytics-api replicas within the NFR threshold. Pure in-process caching is no longer compliant. (2) Added a minimum authentication contract to §11 Assumptions — the auth layer must provide `actor_id`, `tenant_id`, and an `is_tenant_admin(tenant_id)` predicate; override-lock and bypass-lock permissions are separate capabilities owned by the Auth PRD. (3) **Changed `metric_catalog` keying from `(tenant_id, metric_key)` to global `metric_key`**. Per-tenant metadata duplication was a tax on a feature v1 forbids (metadata writes are migration-only). Threshold table remains per-tenant — it is local policy, not product-level metadata. An additive per-tenant metadata-override table can land later if the "per-tenant localization" use case ever becomes real. (4) Added `lock_expires_at` nullable timestamp column to `metric_threshold` with a v1 CHECK constraint enforcing `NULL`; expiry semantics ship in a follow-on when a concrete temporal-lock FR lands. Avoids a breaking migration later. (5) Promoted `source_tag` to `source_tags: string[]` in the API contract (`GET /catalog/metrics` response shape). Storage shape (JSON column vs join-table) remains a DESIGN question, but the API is forward-compatible. Also closed the `product-default` locks OQ with an explicit answer: product-default locks are migrations-only, there is no runtime override mechanism; legitimate legal-override requests flow through backend code migrations.
- **v1.6**: Tightened `is_locked` lock placement to `product-default` and `tenant` scopes only in v1 — locks at `role` / `team` / `team+role` are deferred until there is a real use case. Promoted audit fields `locked_by` / `locked_at` / `lock_reason` from Open Question to required columns in `cpt-metric-cat-fr-threshold-storage`, because emitting a structured "lock-bypass attempt" audit event (the new compliance FR `cpt-metric-cat-fr-threshold-lock-bypass-audit`) requires knowing who authored the lock and when — the event would be incomplete without those fields. Added a `p2` FR `cpt-metric-cat-fr-threshold-lock-bypass-audit` that emits a structured audit event on every 403 `threshold_locked` response, so security / compliance teams can see who tried to bypass locks. Added `locked_at` to the 403 error body. Added OQ on lock granularity (row-level atomic vs per-field), with v1 default being atomic. Added OQ on audit-log destination (analytics-api structured logs vs a dedicated `threshold_lock_audit` table vs both) and retention.
- **v1.5**: Added `is_locked` mechanism on `metric_threshold` rows to support compliance use cases (regulated tenants pinning a metric at a given scope so narrower scopes cannot override). `is_locked` is a `p2` FR on the write side and a resolution-logic rule on the read side. Resolution walks least-specific to most-specific and stops at the first locked row; a tenant admin locking the `tenant` row blocks team/role/team+role overrides for that metric in that tenant. Write CRUD rejects narrower-scope writes that would be shadowed by a locked row with a structured `403 threshold_locked` error identifying the blocking scope. `is_locked` is **not** exposed in the v1 `GET /catalog/metrics` response — Dashboard Configurator does not need it for rendering, and admin UI is out of v1 scope. It becomes an additive field when admin UI ships. Governance questions (who can set locks, audit fields, lock expiry, product-default locks) tracked as Open Questions.
- **v1.4**: Narrowed v1 scope to what actually unblocks Dashboard Configurator and removes the frontend-backend metadata duplication. **Calculation rules, invariant tests, `primary_query_id` linkage, admin-diagnostics endpoint, and reverse-lookup endpoint are deferred out of v1** — pushed to a follow-on revision (or to the upcoming silver-plugin-manifest workstream) once there is a real consumer that reads them. The v1 calculation-rule pitch over-promised readability it could not deliver without exposing the silver/gold dependency chain, which lives outside the catalog boundary. **Reshaped the threshold-scope model to match real axes of variance: `team + role → team → role → tenant → product-default`**. All four scopes ship in v1 (not follow-on) because role-level and team-level threshold variance is a motivating v1 use case — "different bars for PMs vs Backend Devs" and "our team's threshold is different" are the actual support calls the catalog exists to solve. `role_slug` and `team_id` are v1 string columns with no FK constraint; the FK onto `role_catalog` / future team-catalog tables lands in a follow-on migration once those tables exist. **The `dashboard` scope from v1.3 is explicitly removed**: it was a proxy for role scope (since dashboards in Dashboard Configurator are keyed by `(view_type, role)`), and once role is a first-class scope, the `dashboard` proxy is redundant. Admins do not naturally think "change threshold on this dashboard"; they think in role / team / company terms. Reframed the catalog as a projection surface: v1 is seeded via migration, but the table shape is kept friendly to future population from silver-plugin manifests (per Roman Mitasov's plugin proposal) without schema change. Added §14 Gap Analysis mirroring Dashboard Configurator's.
- **v1.3**: Extended the threshold scope model to include `team` as a first-class scope, with precedence `team + dashboard → team → dashboard → tenant → product-default`. Role-scoped overrides remain implicit — they happen through the `dashboard` scope because dashboards are already keyed by `(view_type, role)` in the Dashboard Configurator; no separate `role` scope is introduced. Team scope lands at `p2` and is gated on the Dashboard Configurator's team-lead customization feature, since without an admin path to edit team-scoped thresholds the column is just shelf-ware.
- **v1.2**: Hardened the rule↔SQL consistency story against drift. Promoted invariant-test enforcement to `p1` and reframed it as executable checks per metric (aggregation, null policy, bounds, grain). Added a forward-looking `p2` target for v2 of the catalog where simple rules compile to SQL (Tier 2 — rule becomes authoritative for a subset of metrics; DSL described in this PRD, implementation deferred). Added explicit linkage between `metric_catalog` entries and `analytics.metrics` query UUIDs — `primary_query_id` column, reverse-lookup endpoint, and a one-shot admin diagnostics endpoint that returns catalog row + thresholds + calculation rule + `query_ref` SQL for a given `metric_key`.
- **v1.1**: Expanded catalog scope to include calculation rules per metric — machine-readable declarative descriptions of how each metric is computed (aggregation function, source reference, grain, null policy, bounds). Calculation rules complement but do not replace the imperative `query_ref` SQL in `analytics.metrics`; they provide documentation, enable admin-UI "how is this calculated?" surfaces, support future validation against the SQL, and keep the door open for alternative compute backends without coupling the catalog to a specific execution engine.
- **v1.0**: Initial PRD extracted from Dashboard Configurator PRD v1.4. Metric Catalog is the single source of truth for metric metadata (label, unit, format, thresholds, source tag). It is a foundation primitive that Dashboard Configurator consumes, and that future products (alerting, reports, admin audits, scheduled digests) can also consume.

## 1. Overview

### 1.1 Purpose

Metric Catalog is a backend-owned, per-tenant registry of two things the product knows about a metric:

- **Semantic metadata** — labels, units, formats, `higher_is_better` semantics, source tags, enable flags.
- **Per-tenant thresholds** — `good` / `warn` / `alert_trigger` / `alert_bad` values for bullet-color and alert evaluation.

It replaces the status quo where metric metadata is duplicated between the frontend (`src/screensets/insight/api/thresholdConfig.ts` with `BULLET_DEFS`, `IC_KPI_DEFS`, and most of `METRIC_KEYS`) and backend seed migrations. The catalog is an independent primitive: its first consumer is Dashboard Configurator, but it is not coupled to any specific visualization surface, and future consumers (alerting, reports, scheduled digests, admin audits) read the same response shape.

**Layer boundary**: the catalog describes metrics **as consumers see them** — the gold-layer outputs plus whatever `analytics.metrics.query_ref` does on top (a thin `SELECT` from a gold view, or an on-the-fly aggregation from silver, or a union — `query_ref` is opaque to the catalog). Silver-layer models (per-connector normalized tables like `class_collab_chat_activity`, `class_comms_events`), the transformations that build gold from silver, and the per-tenant customizations that live in those transformations are all **below** the catalog boundary. The catalog names metrics and carries their display / threshold metadata; it does not document the lineage from bronze or silver to the value a user reads.

**Explicit non-goal in v1**: the catalog does **not** describe how a metric is computed. Calculation-rule storage, machine-readable aggregation / grain / null-policy descriptors, rule↔SQL consistency enforcement, `primary_query_id` linkage, and the admin diagnostics endpoint are deferred to a follow-on revision. The "how is this calculated?" problem is real, but it lives deeper in the silver/gold dependency chain (including per-tenant silver-model customizations) and cannot be solved by a descriptive field on a catalog row without its own design pass. See §4.2 and §13.

### 1.2 Background / Problem Statement

Insight's analytics-api today stores only `analytics.metrics` (`id`, `query_ref`, `name`, `description`) and an empty `analytics.thresholds` table. Semantic metadata about each metric — the label a user reads, the unit suffix, whether higher is better, the `good`/`warn` thresholds that drive bullet color — lives on the frontend, hardcoded in TypeScript. Three consequences:

- **Drift**: a metric rename or a unit change requires synchronized edits in two repos; the product has already seen cases where a backend UUID added a new row without a frontend entry and the bullet rendered with generic fallbacks.
- **Per-tenant threshold tuning is impossible without a frontend deploy**: a compliance team that wants `focus_time_pct` target stricter than engineering has no path shorter than a code change.
- **Other potential consumers are blocked**: any future surface that needs to know "what does metric `cc_active` mean, what unit, what's good" — alerting rules, scheduled digests, admin audit UI — would either duplicate the metadata again or consume the current frontend file through a strange back-channel.

The related "how is this calculated?" opacity — calculation logic living exclusively in ClickHouse SQL inside `query_ref`, routed through a dependency chain across silver and gold views that may be tenant-specific — is a real pain, but it is **not** solved by this PRD. Capturing it would require exposing the silver/gold layering and per-tenant customizations through some form of plugin manifest or lineage graph, which is its own design. v1 leaves `query_ref` as the authoritative source for "how" and treats the catalog as a metadata/threshold surface only.

**Target Users**:

- Tenant admins tuning thresholds and labels for their organization without waiting for engineering
- Insight product team seeding default metric metadata as part of product releases
- Downstream backend services (Dashboard Configurator today; alerting and reports tomorrow) that need metadata about metrics to render or evaluate them

**Key Problems Solved**:

- No single source of truth for metric labels, units, and thresholds
- No way to override thresholds per tenant without a code deploy
- No discoverable registry of "what metrics exist and what do they mean" for tools and humans

### 1.3 Goals (Business Outcomes)

**Success Criteria**:

- 100% of metric metadata currently duplicated between frontend and backend resolved from a single MariaDB source (Baseline: duplicated in `thresholdConfig.ts` and Rust seed migrations; Target: backend-only by end of rollout)
- Zero frontend deploys required to change a label, unit, or threshold on existing metrics (Baseline: required for every change; Target: 0)
- Threshold changes propagate to the UI within one page load after the DB update (Baseline: not supported; Target: ≤ 5-minute cache TTL)
- First non-dashboard consumer (e.g., an alerting rule authored against the catalog) ships without any catalog-schema change

**Capabilities**:

- Persist one row per metric with rich metadata (label, sublabel, description, unit, format, higher_is_better, is_member_scale, source tag, enable flag)
- Persist tenant-scoped threshold overrides (`good`, `warn`, `alert_trigger`, `alert_bad`)
- Serve the catalog to consumers via a cacheable read endpoint
- Validate metadata changes at the database and API layers (no silent corruption)

### 1.4 Glossary

| Term | Definition |
|------|------------|
| Metric | A named, quantitative measurement produced by a `metric_query` that returns rows annotated with one or more `metric_key` values. Example: `cursor_active`, `tasks_completed`, `ai_loc_share2`. |
| Metric key | Stable string identifier for a metric, used to cross-reference metadata and threshold rows. Follows `snake_case` convention. |
| Metric catalog | The MariaDB table that persists one row per `metric_key` with semantic metadata. Product-owned metrics are global (`tenant_id IS NULL`); tenant-custom metrics (defined by individual tenants) are an additive extension path through a separate `metric_catalog_tenant_override` table — out of v1 scope, see §13 OQ for the resolution. Per-tenant thresholds live in a separate `metric_threshold` table. |
| Metric threshold | A configurable boundary attached to a metric at some scope — company-wide (`tenant`), per-role (`role`), per-team (`team`), per-team-per-role (`team+role`), or the product-seeded floor (`product-default`). Thresholds are **optional and multi-row**: a metric may carry zero, one, or many threshold rows (different visual zones, multiple alert levels, or both). Each row carries a kind (e.g., `good`, `warn`, `bad`, `alert`), a numeric value, and a human-readable label so admins can distinguish multiple thresholds on the same metric. A metric without any matching row in any scope renders as a plain number with no status color and no alerting. |
| Threshold scope | One of `product-default` (seeded floor), `tenant` (per-company default), `role` (per-role override within a tenant), `team` (per-team override within a tenant), or the composite `team+role` (most specific: this team's take on this role's bar). Resolution precedence, most specific wins: `team+role → team → role → tenant → product-default`. All five ship in v1. A locked row (`is_locked = true`) at a broader scope bounds the walk — narrower scopes are ignored. See `cpt-metric-cat-fr-scoped-thresholds`. |
| Role | A per-tenant professional-function slug (e.g., `backend-dev`, `qa`, `pm`), originating from Dashboard Configurator's `role_catalog`. v1 of this PRD carries `role_slug` as a VARCHAR reference without FK; the constraint lands in a follow-on migration. The catalog's `role` scope applies thresholds to everyone carrying the matching `role_slug` in the caller's request context. |
| Team | A per-tenant team identifier (`team_id`, VARCHAR in v1). Source of truth and exact shape tracked as a DESIGN question; v1 carries string references to match whatever convention Identity Resolution exposes. |
| Dashboard scope (rejected) | Not a threshold scope in this catalog. An earlier v1.3 model had `dashboard` as a scope; v1.4 removed it as a redundant proxy for `role` (Dashboard Configurator keys dashboards by `(view_type, role)`). Admins tuning thresholds think in role / team / company terms, not per-dashboard. Listed here only to mark its explicit rejection for reviewers looking for it. |
| Source tags | An array of short strings identifying the connectors / ingestion origins a metric depends on (e.g., `["cursor"]`, `["m365", "zoom", "slack"]`, `["bamboohr"]`). Drives availability checks and connector-readiness diagnostics. The API contract is `source_tags: string[]` from v1; storage shape (JSON column vs join-table `metric_source`) is a DESIGN question. Most metrics have a single-element array; genuinely multi-source metrics (e.g., focus time derived from M365 calendar + Zoom + Slack) use multi-element arrays without breaking the contract. |
| Query ref | The existing ClickHouse SQL in `analytics.metrics.query_ref`, keyed by UUID, that imperatively produces the metric's rows at query time. Reads from whatever layer it needs — a thin `SELECT` over a gold view, an on-the-fly aggregation from silver, a union across silver and gold. The catalog is agnostic to what `query_ref` does; it trusts `query_ref` to produce the named `metric_key` values. One `query_ref` can return multiple metric keys. Remains the authoritative description of how a metric is computed in v1. |
| Gold layer | The aggregated, denormalized ClickHouse views (`gold.*`) that serve as the typical read layer for metric queries. Consumer-facing metric values usually come from here. Within the catalog's scope. |
| Silver layer | Per-connector cleaned and normalized tables (`class_*`, `raw_*`) that feed the gold layer. Below the catalog boundary. Silver-layer lineage, aggregation, and per-tenant customization are out of scope; see §4.2. |

## 2. Actors

### 2.1 Human Actors

#### Tenant Admin

**ID**: `cpt-metric-cat-actor-tenant-admin`

**Role**: Edits the tenant's metric thresholds through the admin API. Reads the catalog to see labels, units, and the currently resolved threshold for each metric. Does not edit metadata in v1 (labels, units, formats) — those are product-team-owned and ship as migrations.

**Needs**: A view of the currently resolved catalog for their tenant, including which thresholds are from tenant defaults vs product-seeded defaults; audit trail of who changed what; preview of the effect of a threshold change before it goes live.

#### Insight Product Team

**ID**: `cpt-metric-cat-actor-product-team`

**Role**: Seeds the canonical metric catalog that every tenant starts with. Ships new metrics as part of feature releases by adding catalog entries in the same PR as the new `metric_query`. Deprecates metrics via the `is_enabled` flag.

**Needs**: Migration-based seed mechanism; a way to add a catalog entry in the same review as the metric query and frontend changes that consume it; a predictable lifecycle for deprecation.

### 2.2 System Actors

#### Analytics API

**ID**: `cpt-metric-cat-actor-analytics-api`

**Role**: Serves the catalog via `GET /catalog/metrics` with resolved per-tenant thresholds. Exposes admin CRUD for thresholds. Validates metadata integrity at the API layer.

#### Catalog Consumer

**ID**: `cpt-metric-cat-actor-consumer`

**Role**: Any backend or frontend component that reads `GET /catalog/metrics` and relies on the returned metadata. In v1 this is Dashboard Configurator; in later waves, it will include alerting, reports, scheduled digests, and the admin UI.

**Needs**: A cacheable, versioned read endpoint; stable field names across additive changes; explicit signal when a metric is disabled so consumers can degrade gracefully.

#### MariaDB Catalog

**ID**: `cpt-metric-cat-actor-mariadb`

**Role**: Persists the `metric_catalog` and `metric_threshold` tables. Provides referential integrity between thresholds and catalog entries.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

None beyond project defaults (Rust, Axum, SeaORM, MariaDB). The catalog inherits the analytics-api service runtime.

## 4. Scope

### 4.1 In Scope

- `metric_catalog` table: one row **per `metric_key`** (all rows global in v1) with fields `tenant_id` (nullable, v1 CHECK forces `NULL`), `label_i18n_key`, `sublabel_i18n_key`, `description_i18n_key`, `unit`, `format`, `higher_is_better`, `is_member_scale`, `source_tags` (array of strings), `is_enabled`, timestamps. Per-tenant metadata overrides on product-owned metrics are out of v1 scope; if ever needed, an additive `metric_catalog_tenant_override` table lands without schema change to `metric_catalog`.
- `metric_catalog.tenant_id` nullable column. **`tenant_id IS NULL` indicates a product-owned global metric available to every tenant; non-NULL values are reserved for tenant-custom metrics in a future revision.** Ships in v1 with a DB CHECK constraint enforcing `tenant_id IS NULL`. In v1 no behavior depends on it; the column reserves the additive extension path for **tenant-custom metrics** (metrics a tenant adds on top of the product-owned catalog, e.g., regulator-mandated KPIs). The follow-on PRD that enables tenant-custom drops the CHECK, adds CRUD for rows with `tenant_id = :caller`, and updates read resolution to `WHERE tenant_id = :caller OR tenant_id IS NULL`. Product-owned rows (`tenant_id IS NULL`) remain the global baseline; tenant-custom is **additive**, never a replacement. See §13 OQ for the query-layer story and §12 Risk for the silver-evolution contract.
- `metric_threshold` table: configurable per-metric, per-scope thresholds. Each row carries a kind, a numeric value, a label, and the scope filters. A metric can carry any number of threshold rows in any scope. Both visual thresholds (rendered as zone colors on dashboards) and alerting thresholds (consumed by the alerting subsystem) live in this single table — distinguished by kind. The `scope` enum's v1 domain is `{ product-default, tenant, role, team, team+role }`. The table carries nullable `role_slug` (VARCHAR) and `team_id` (VARCHAR) columns with **no FK constraint in v1** — they are string references matching the per-tenant `role_catalog.role_slug` convention in Dashboard Configurator and the team-identifier convention used by Identity Resolution. FK constraints land in a follow-on migration after `role_catalog` / team-catalog tables exist. A `is_locked` boolean (default `false`) on each row allows a broader-scope authority to pin the resolution: when a locked row exists at some scope, rows at narrower scopes are ignored during resolution. Resolution precedence (most specific wins, subject to locks): `team + role → team → role → tenant → product-default`. Concrete schema (column types, kind enum values, label uniqueness rules, optional callback for alert kinds) lands in DESIGN.
- `GET /catalog/metrics` read endpoint returning catalog + resolved thresholds for a request's `(tenant, role, team)` context
- `POST/PUT/DELETE /v1/admin/metric-thresholds` admin CRUD on thresholds at any v1 scope
- Seed migration importing the current frontend metadata (`BULLET_DEFS`, `IC_KPI_DEFS`, most of `METRIC_KEYS`) as the initial catalog. Seed rows come from a migration; the design keeps the door open for future population from silver-plugin manifests without schema change (see §13).
- Deletion of the duplicated metadata on the frontend once the catalog endpoint is live
- Decision on the existing empty `analytics.thresholds` table (repurpose vs drop) — tracked in Open Questions, resolved in DESIGN. This PRD is the canonical owner of that decision; Dashboard Configurator PRD references it but does not re-own it.
- Cache layer (configurable TTL, default 5 minutes) fronting `GET /catalog/metrics`

### 4.2 Out of Scope

- Admin UI for editing the catalog — deferred to a follow-up Admin-UI PRD; this PRD covers data model and API
- **Calculation rules** — machine-readable descriptors of aggregation, source, grain, null policy, bounds, or formula. Deferred to a follow-on revision (or to the silver-plugin-manifest workstream) once there is a real consumer that reads these fields and a decision on where calculation metadata lives (catalog row vs plugin manifest vs dbt lineage). In v1, `query_ref` remains the authoritative source for "how a metric is computed".
- **`primary_query_id` linkage** between catalog rows and `analytics.metrics.id` — deferred with calculation rules. Reverse lookup and one-shot admin diagnostics endpoints that depend on this linkage are out of scope in v1.
- **Invariant tests** that validate `query_ref` behavior against declared calculation rules — out of scope in v1 because calculation rules are out of scope; fixture-framework infrastructure is not landed on speculation.
- **Rule-as-authoritative SQL compilation** — previously tracked as a v2 target; remains a possible future direction, now explicitly outside the v1 catalog's scope.
- Tenant-level edits of semantic metadata (labels, units, formats) in v1 — keeping the product team as the owner of metadata avoids drift across tenants and keeps i18n manageable
- **Tenant-specific custom metrics (behavior)** — a metric that exists for one tenant only (e.g., a bank's regulator-mandated `mas_compliance_score`, a healthcare tenant's HIPAA-driven KPI). CRUD endpoints, query-layer storage, formula DSL, read-path resolution over a mixed global + tenant set — all deferred to a **follow-on PRD**. v1 does **not** expose any way to create `tenant_id IS NOT NULL` rows; the DB CHECK constraint rejects them. What v1 **does** ship is the schema slot: nullable `tenant_id` column, pre-wired so the follow-on lands additively without a breaking migration. Path (a) from the v1.9 §13 OQ (nullable `tenant_id` on `metric_catalog`) is chosen over path (b) (separate `metric_catalog_tenant_custom` table) — see Changelog v1.10. Query-layer (SQL rows in `analytics.metrics` vs a formula DSL poverh existing metrics) and disable-for-me semantics (tenant hides a product-owned metric for themselves) are tracked as §13 OQs for the follow-on PRD to answer
- **Tenant edits of product-owned metrics** — a tenant editing the `query_ref`, labels, units, or thresholds-semantics of a metric the product ships (as opposed to adding their own). v1 keeps product-owned metrics as the global contract (`cpt-metric-cat-fr-metadata-writes`); tenant-custom is **additive**, never a replacement. Follow-on work does not relax this — if a tenant wants to hide a product metric they consider incorrect, the disable-for-me slot (§13 OQ) is the designed escape hatch, not an edit surface
- **A dashboard-scoped threshold** ("this dashboard has its own threshold regardless of who views it") — explicitly **rejected**, not deferred. Admins do not think in dashboard terms when tuning thresholds; they think in role / team / company terms, all of which are first-class v1 scopes. Keeping a `dashboard` scope would only add an override path that duplicates role scope (because Dashboard Configurator keys dashboards by `(view_type, role)`). If a future use case genuinely requires dashboard-level overrides independent of role, it is revisited in an Open Question, not added by default.
- FK constraints on `role_slug` / `team_id` in v1 — v1 ships these as string references; FK is added in a follow-on migration once the referenced tables (`role_catalog` from Dashboard Configurator, team-catalog TBD) exist. String-only v1 is deliberate: it unblocks role / team variance from day one without blocking on Dashboard Configurator.
- Conditional / filtered / formula-based thresholds (e.g., "only count commits with LOC ≤ 5K") — v1 thresholds are numeric scalars (`good` / `warn` / `alert_trigger` / `alert_bad`). Richer threshold expressions are a future direction, tracked in Open Questions
- Alerting rule engine — a future consumer of the catalog, not part of this PRD
- Cross-tenant catalog sharing or federation
- Soft delete / audit log of metadata changes beyond timestamps
- Metric `query_ref` storage — stays in the existing `analytics.metrics` table untouched by this PRD
- **Silver-layer model descriptions and bronze-to-silver / silver-to-gold transformation metadata** — the catalog sits at the gold / consumer layer (see §1.1 "Layer boundary"). Per-connector silver models, their dbt transformations, staging schemas, and per-tenant silver customizations are explicitly outside this PRD's domain. They are a natural fit for silver-plugin manifests (see §13) but not for a catalog row.

## 5. Functional Requirements

### 5.1 Catalog Storage

#### Catalog Persists Semantic Metadata per Tenant per Metric

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-catalog-storage`

The system **MUST** persist rows in `metric_catalog` with at least: `tenant_id` (nullable, v1 CHECK enforces `IS NULL`), `metric_key`, `label_i18n_key`, `sublabel_i18n_key`, `description_i18n_key`, `unit`, `format`, `higher_is_better`, `is_member_scale`, `source_tags` (array of strings), `is_enabled`, `created_at`, `updated_at`. In v1, because of the CHECK, every row has `tenant_id IS NULL` and represents a global product-owned metric — functionally "one row per `metric_key`", identical to v1.9 behavior. The **primary key evolution strategy** (keep `PK = metric_key` for v1 and rebuild when tenant-custom lands vs ship v1 with synthetic `id BIGINT AUTO_INCREMENT` + `UNIQUE(tenant_id, metric_key)` treating NULL-global and non-NULL tenant rows as distinct) is a DESIGN question — see §13 OQ. Either option produces identical v1 external behavior. In v1 rows are created by seed migrations; the table shape is intentionally friendly to future population from an external source (e.g., silver-plugin manifests registering their metrics at install time) without requiring a schema change.

**Rationale**: Product-owned metadata (labels, units, format, `higher_is_better`) is a global contract — identical across tenants by design (see `cpt-metric-cat-fr-metadata-writes`). Giving every product-owned row `tenant_id IS NULL` avoids duplicating 200 metrics × 1000 tenants = 200 000 identical rows, which would be a tax on a capability v1 forbids. The **nullable** column (with v1 CHECK) reserves the additive extension path for tenant-custom metrics (rows with `tenant_id = :caller`) without requiring a breaking migration later — mirroring the `lock_expires_at` pattern. Architectural symmetry with thresholds: product-owned global baseline + tenant-owned additive overlay, same resolution philosophy applied one layer up. If per-tenant metadata overrides on **product-owned** metrics ever become a real use case (tenant admin rewriting a label, per-tenant localization), they land through a separate additive `metric_catalog_tenant_override` table without modifying `metric_catalog` — that is a distinct extension from tenant-custom and is **not** the same slot. Per-tenant **thresholds** remain per-tenant because thresholds are local policy, not product contract — they live in `metric_threshold`. Seeding-via-migration in v1 does not lock the catalog into being an admin-edited free-form store; populating rows from declarative plugin manifests later is an additive change on the write side only.

**Actors**: `cpt-metric-cat-actor-analytics-api`, `cpt-metric-cat-actor-mariadb`

#### Catalog Enable Flag Controls Visibility

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-enable-flag`

The system **MUST** honor `is_enabled = false` by excluding the row from `GET /catalog/metrics`. Existing consumers (e.g., dashboards) referencing a disabled metric **MUST** continue to work — the reference resolves as absent rather than erroring — so downstream systems can tolerate deprecations without coordinated rollbacks.

**Rationale**: Deprecation lifecycle needs a mechanism that does not break production surfaces the moment a row is disabled.

**Actors**: `cpt-metric-cat-actor-consumer`

### 5.2 Threshold Storage and Resolution

#### Threshold Storage Shape

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-threshold-storage`

The system **MUST** persist thresholds in `metric_threshold` with at least the following columns: `id` (PK), `tenant_id` (nullable only for `product-default` scope), `metric_key`, `scope`, `role_slug` (nullable VARCHAR), `team_id` (nullable VARCHAR), `good`, `warn`, `alert_trigger` (nullable), `alert_bad` (nullable), `is_locked` (boolean, default `false`), `locked_by` (nullable VARCHAR — actor identifier), `locked_at` (nullable timestamp), `lock_reason` (nullable VARCHAR — free-text justification), `lock_expires_at` (nullable timestamp), `created_at`, `updated_at`. The `scope` column is an enum over `{ product-default, tenant, role, team, team+role }`. The `(tenant_id, metric_key, scope, role_slug, team_id)` tuple **MUST** be unique. `role_slug` / `team_id` are **not** FK-constrained in v1; they hold string values that the consumer supplies in request context. The follow-on migration that introduces FK constraints on them is additive and does not rewrite existing rows. `is_locked = true` in v1 is only permitted on rows with `scope ∈ { product-default, tenant }`; a CHECK constraint or equivalent enforces this. `locked_by` / `locked_at` / `lock_reason` are populated when `is_locked = true` and reset to NULL when the lock is cleared. `lock_reason` **MUST** be NOT NULL whenever `is_locked = true`, enforced via DB CHECK constraint `(is_locked = false OR lock_reason IS NOT NULL)`; write CRUD rejects lock-set requests without `lock_reason`. `lock_expires_at` is reserved for a future temporal-lock FR: v1 **MUST** enforce `lock_expires_at IS NULL` via CHECK constraint, so the column ships in v1 schema without v1 behavior; the follow-on FR that introduces expiry semantics drops the CHECK constraint additively.

**Rationale**: Four real axes of variance (company / team / role / product floor) plus the composite `team + role` cover the actual support calls the catalog exists to solve: "PMs need a different bar than Backend Devs" (role), "our team's standard is different" (team), "our company has its own compliance targets" (tenant). Shipping these as v1 scopes is a deliberate reversal of the earlier "tenant only in v1, role/team later" narrowing — role / team variance is a motivating v1 use case, not a follow-on. String-based `role_slug` / `team_id` avoid blocking on `role_catalog` and future team-catalog tables without sacrificing the write path.

**Actors**: `cpt-metric-cat-actor-tenant-admin`, `cpt-metric-cat-actor-product-team`, `cpt-metric-cat-actor-mariadb`

#### Tenant-Default and Product-Default Fallback

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-tenant-thresholds`

For every `metric_key` that is `is_enabled = true`, the system **MUST** return a non-null resolved threshold for any `(tenant, role, team)` request context. When no narrower-scope row matches, the system **MUST** fall back to the `tenant`-scoped row; when that is absent, it **MUST** fall back to `product-default`. `product-default` rows **MUST** exist for every seeded metric so the resolution chain is never empty.

**Rationale**: Every metric must be colorable out of the box for every tenant regardless of how sparse their per-tenant customization is. Product-default as a mandatory floor removes the "unseeded metric renders with no bullet color" failure mode.

**Actors**: `cpt-metric-cat-actor-consumer`, `cpt-metric-cat-actor-product-team`

#### Scoped Threshold Resolution

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-scoped-thresholds`

The system **MUST** resolve thresholds for a request with context `(tenant_id, role_slug?, team_id?)` using the following algorithm, which is a most-specific-wins walk bounded by locks:

1. Collect matching rows from least-specific to most-specific, visiting each of `product-default`, `tenant`, `role`, `team`, `team+role` in turn. A scope matches only when all of its required context fields (e.g., `role_slug` for `role`, both `role_slug` and `team_id` for `team+role`) are supplied in the request and match the row's values.
2. For each matching row in that order, append it to an ordered list of candidates. If the row has `is_locked = true`, **stop walking** — no narrower-scope rows are considered.
3. Return the last (most-specific) row in the candidate list.

Equivalent pseudocode:

```
candidates = []
for scope in [product-default, tenant, role, team, team+role]:
    row = find_matching_row(scope, request_context)
    if row is None:
        continue
    candidates.append(row)
    if row.is_locked:
        break
return candidates[-1]   # most specific within the lock ceiling
```

Resolution is canonical and lives in the catalog; consumers **MUST NOT** reimplement it. Requests **MAY** omit `role_slug` and / or `team_id`; missing context skips the scopes that require it. Admin writes **MUST** populate `scope` explicitly and **MUST** carry the matching `role_slug` / `team_id` values (non-null for scopes that need them; null for scopes that do not).

**Implementation note**: for `GET /catalog/metrics` (which resolves across all enabled metrics in one call), the expected query shape is a single bulk fetch of all candidate rows for the tenant (plus `product-default`) scoped to the requested `(role_slug, team_id)` context, followed by an in-memory walk per metric. A naive N+1 approach (one query per metric × five scopes = up to 1000 round trips for 200 metrics) would fail `cpt-metric-cat-nfr-read-latency`. Composite indexing on `(tenant_id, metric_key, scope)` plus partial indexes for narrower scopes is expected; exact index shape is DESIGN.

**Rationale**: A fixed precedence chain authored by the catalog (not by consumers) is the only way every surface in the product agrees on which threshold wins. The chain orders scopes from most-specific to least-specific authority over a given `(tenant, role, team)` context: `team + role` is the most specific (this team's PMs specifically); `team` is next (this team, across all roles); `role` below team (company-wide PMs); `tenant` beneath role (company default); `product-default` is the unconditional floor. Team above role means a team lead's call on their team's bar wins over a role-level company standard — which matches the reality that team leads are the ones accountable for their team's outcomes. If the role-level standard must be honored for a specific team's role, the team lead has to explicitly match it at `team + role`, not at `team`. The removed `dashboard` scope from v1.3 would have been a proxy for role and is now redundant. The `is_locked` flag gives broader-scope authorities a way to pin resolution for compliance-sensitive metrics — a regulated tenant setting `is_locked = true` on the `tenant` row for `security_vuln_count` means teams cannot silently diverge, closing the "team override violates compliance" gap without requiring RBAC surgery.

**Actors**: `cpt-metric-cat-actor-consumer`, `cpt-metric-cat-actor-tenant-admin`

### 5.3 Read API

#### GET /catalog/metrics Returns Fully Resolved Catalog

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-read-endpoint`

The system **MUST** expose `GET /catalog/metrics` returning, for the caller's tenant, every `is_enabled = true` catalog row joined with its resolved thresholds per `cpt-metric-cat-fr-scoped-thresholds`. The endpoint **MUST** accept optional query parameters `role_slug` and `team_id`; when present, they participate in threshold resolution. When absent, resolution falls back to the `tenant` / `product-default` chain. The response **MUST** include a `tenant_id` echo and a `generated_at` timestamp to support client-side caching. The response **MUST** also include, per metric, the scope that supplied the resolved threshold as `resolved_from: "team+role" | "team" | "role" | "tenant" | "product-default"` so consumers and admin tools can explain which row won without a second request.

**Rationale**: Consumers need one call to hydrate everything they need about every metric they will render or evaluate. Round-trips per metric would be unacceptable. Surfacing the winning scope by default costs a single short string per metric and closes the "why is this color different here" debugging loop without a second endpoint. The query parameters are optional so generic catalog hydrators (e.g., an admin audit UI listing the tenant-wide state) keep working without knowing about role or team.

**Actors**: `cpt-metric-cat-actor-consumer`, `cpt-metric-cat-actor-analytics-api`

#### Read Endpoint Is Cacheable

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-cache`

The system **MUST** front the read endpoint with a cache layer whose default TTL is 5 minutes and whose cache key includes the tenant identifier. Admin writes **MUST** invalidate the cache for the affected tenant so threshold changes appear on the next page load. **The cache implementation (backend in-process, shared Redis, gateway-level, CDN-level) is a DESIGN decision tracked in §13 OQ; this FR mandates that a cache exists and that its invalidation correctly tracks admin writes — not its physical location.**

**Rationale**: The catalog is high-read low-write; stale reads beyond a few minutes are acceptable but stale reads after an admin write are not — they produce the "I changed the threshold, nothing happened" support call.

**Actors**: `cpt-metric-cat-actor-analytics-api`

### 5.4 Admin Write API

#### Threshold CRUD Endpoints

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-threshold-crud`

The system **MUST** expose for tenant admins:

- `GET /v1/admin/metric-thresholds` — list, with filters by `metric_id`, `scope`, `tenant_id`, `role_slug`, `team_id`.
- `GET /v1/admin/metric-thresholds/:id` — single row.
- `POST /v1/admin/metric-thresholds` — create.
- `PUT /v1/admin/metric-thresholds/:id` — update.
- `DELETE /v1/admin/metric-thresholds/:id` — delete.

Writes **MUST** enforce (a) authorization checking that the caller is a tenant admin for the target tenant, (b) referential integrity with `metric_catalog`, (c) sanity bounds appropriate to the threshold kind (e.g., `warn` not crossing `good` in the wrong direction relative to `higher_is_better`), (d) scope-shape validity (right combination of `role_slug` / `team_id` for the declared `scope`).

**Rationale**: Admins need a supported way to change thresholds. Validation prevents the "I accidentally set good below warn" class of mistakes from reaching the UI.

**Actors**: `cpt-metric-cat-actor-tenant-admin`, `cpt-metric-cat-actor-analytics-api`

#### Lock Enforcement on Writes

- [ ] `p2` - **ID**: `cpt-metric-cat-fr-threshold-lock`

The system **SHOULD** support pinning a threshold row with `is_locked = true` so that rows at narrower scopes are ignored during resolution per `cpt-metric-cat-fr-scoped-thresholds`. Write CRUD **MUST** enforce the following:

- `is_locked = true` is permitted only on rows with `scope ∈ { product-default, tenant }` in v1. Locks at `role` / `team` / `team+role` are out of v1 scope; requests attempting them are rejected with a structured error. Widening is revisited when a concrete use case lands.
- When an admin attempts to create or update a row at some scope `S` for a `(tenant, metric_key)` pair, and a row at a **broader** scope has `is_locked = true` such that scope `S` would be shadowed by the lock during resolution, the request **MUST** be rejected with HTTP 403 and an error body of shape `{ error: "threshold_locked", blocking_scope: "<scope>", blocking_row_id: "<uuid>", locked_at: "<ISO-8601 timestamp>" }`. Only admins with explicit authorization to override a lock at the blocking scope (modelled separately in RBAC) can bypass.
- Setting `is_locked = true` on a row **MUST** be a privileged write: only admins at or above the scope's authority may set it (e.g., tenant-scope locks require tenant-admin privileges; `product-default` locks are settable only through seed migrations). The exact RBAC mapping is an Open Question.
- Setting `is_locked = true` **MUST** populate `locked_by` (actor identifier from the authenticated session), `locked_at` (server timestamp), and **`lock_reason` (non-empty string justification provided by the caller)**. Lock-set requests missing `lock_reason` **MUST** be rejected with HTTP 400 and `{ error: "lock_reason_required" }`. The DB-level CHECK constraint in `cpt-metric-cat-fr-threshold-storage` is the ultimate backstop.
- Clearing a lock (`is_locked = true` → `false`) **MUST** require the same privilege as setting it and **MUST** reset `locked_by` / `locked_at` / `lock_reason` to NULL.

**Rationale**: Lock is the compliance mechanism that RBAC alone does not cleanly express. A regulated tenant setting `is_locked = true` on `security_vuln_count` at `tenant` scope is auditably enforcing a company-wide standard; team leads attempting to relax the bar for their team get a clear 403 with the blocking scope identified, instead of a silent 200 and an inspection-time surprise. Restricting v1 locks to `product-default` / `tenant` scopes keeps the mental model simple: "product or tenant authority pins a value, narrower scopes cannot override".

**Actors**: `cpt-metric-cat-actor-tenant-admin`, `cpt-metric-cat-actor-analytics-api`

#### Lock-Bypass Audit Events

- [ ] `p2` - **ID**: `cpt-metric-cat-fr-threshold-lock-bypass-audit`

When a write to `metric_threshold` is rejected with `403 threshold_locked` per `cpt-metric-cat-fr-threshold-lock`, the system **MUST** emit a structured audit event containing at least: `actor_id` (who attempted the write), `tenant_id`, `metric_key`, `attempted_scope`, `attempted_values` (the `good` / `warn` / etc. the admin tried to set), `blocking_scope`, `blocking_row_id`, `locked_by` (author of the blocking lock), `locked_at` (when the blocking lock was set), and the event's own timestamp. Every event **MUST** be persisted to **both**:

- The analytics-api **structured log stream** — for real-time observability, alerting, and operational dashboards.
- A dedicated **`threshold_lock_audit` MariaDB table** (columns: `id`, `event_type` enum `{ bypass_attempt, lock_set, lock_cleared }`, `actor_id`, `tenant_id`, `metric_key`, `attempted_scope` nullable, `attempted_values` JSON nullable, `blocking_scope` nullable, `blocking_row_id` UUID nullable, `locked_by` nullable, `locked_at` nullable, `lock_reason` nullable, `event_at` NOT NULL, `created_at` NOT NULL) — for long-term audit retention and queryable export from a future admin UI.

The table **MUST** support retention of at least 1 year out of the box; tenants with stricter regulatory cycles (banking, healthcare) may require longer. Exact retention policy is an Open Question. A successful lock set or clear **MUST** also emit a corresponding `lock_set` / `lock_cleared` event to both sinks, so the full lifecycle is visible.

**Rationale**: Regulated tenants need a "who tried to bypass our compliance thresholds" audit trail that survives long enough to matter. Relying on structured logs alone is fragile — Loki / ELK / equivalent commonly default to 30–90 day retention, while compliance cycles often require 1–7 years. A dedicated table gives queryable persistence independent of log-infrastructure choices, enables future CSV / JSON export from the admin UI, and decouples audit retention from log-cost economics. Logs continue to exist for real-time operational use. This requirement is what promoted `locked_by` / `locked_at` from Open Question to required fields in `cpt-metric-cat-fr-threshold-storage`: an audit event that says "someone tried to bypass a lock" is incomplete without attribution of the lock itself.

**Actors**: `cpt-metric-cat-actor-analytics-api`, `cpt-metric-cat-actor-tenant-admin`

#### Metadata Writes Are Migration-Only in v1

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-metadata-writes`

The system **MUST NOT** expose a runtime admin endpoint to edit `metric_catalog` metadata (labels, units, formats, `higher_is_better`). Metadata changes **MUST** ship as backend code migrations reviewed through the normal release process. Disabling a metric via `is_enabled = false` **MAY** be exposed to admins in a future follow-up.

**Rationale**: Metadata drift across tenants would fragment comparability of the product. Letting a single tenant unilaterally rename `tasks_closed` to "Story Points Completed" would break cross-tenant comparisons and i18n. Thresholds are local policy; metadata is product-level contract.

**Actors**: `cpt-metric-cat-actor-product-team`

### 5.5 Seed and Migration

#### Seed from Frontend Metadata

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-seed-from-frontend`

The system **MUST** seed the initial catalog by importing the metadata currently hardcoded in `src/screensets/insight/api/thresholdConfig.ts` (`BULLET_DEFS`, `IC_KPI_DEFS`) and `src/screensets/insight/types/index.ts` (`METRIC_KEYS`). The seed is a one-time export that the product team reviews before merging. After the seed lands and the frontend consumes `GET /catalog/metrics`, the duplicated metadata on the frontend **MUST** be removed. The rollout **SHOULD** go through a short transitional phase where the frontend fetches the catalog but retains local metadata as a fallback; the removal of the fallback ships in the immediately following release. Byte-for-byte comparison between rendered output before and after migration is a rollout gate.

**Rationale**: Zero-downtime migration: on day one, the catalog returns the same values the frontend was hardcoding, so no visible change for end users. The transitional-fallback phase gives a revert-path if a subtle divergence slips past the comparison test, and costs only one extra release to remove.

**Actors**: `cpt-metric-cat-actor-product-team`

#### Existing Empty `analytics.thresholds` Table Resolved

- [ ] `p1` - **ID**: `cpt-metric-cat-fr-thresholds-table-resolution`

The system **MUST** explicitly resolve the fate of the existing empty `analytics.thresholds` table in the same migration series that introduces `metric_threshold`. Options: (a) rename `thresholds` → `metric_threshold` and extend the schema, (b) drop `thresholds` and create a fresh `metric_threshold`, (c) keep `thresholds` as a generic threshold store and reference it from the catalog. The chosen option is tracked in Open Questions and selected in DESIGN; leaving the ambiguity unresolved is not allowed.

**Rationale**: Orphaned empty tables with unclear purpose cause future confusion. Picking a path now keeps the migration file honest.

**Actors**: `cpt-metric-cat-actor-product-team`

#### Periodic Integrity Check for Role / Team References

- [ ] `p2` - **ID**: `cpt-metric-cat-fr-integrity-check`

A periodic background job **SHOULD** scan `metric_threshold` for rows whose `role_slug` no longer matches any live `role_catalog.role_slug` entry in the row's tenant, or whose `team_id` no longer matches any live entry in the future team-catalog. Orphaned rows **MUST** be surfaced through an admin diagnostics view or a notification channel (exact form is a DESIGN question). The job **MUST NOT** auto-delete or auto-rewrite orphaned rows in v1 — flagging only, with human action required.

This FR is gated on Dashboard Configurator delivering `role_catalog` (no canonical list to check against until then). v1 of this PRD ships **without** the integrity check because the check has nothing to run against; the follow-on migration that lands FK constraints on `role_slug` / `team_id` is the same release that turns this FR on. Implementation is a real backlog ticket, not documentation of a hypothetical safeguard.

**Rationale**: Shipping `role_slug` / `team_id` as unconstrained strings in v1 is a deliberate trade-off (per `cpt-metric-cat-fr-threshold-storage`) that keeps the catalog's v1 decoupled from Dashboard Configurator's tables. The trade is only safe if there is a real, scheduled mechanism that catches orphaned references once the canonical lists exist — otherwise `metric_threshold` becomes a graveyard of dangling thresholds after the first role rename or team dissolution.

**Actors**: `cpt-metric-cat-actor-analytics-api`, `cpt-metric-cat-actor-tenant-admin`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### Performance — Catalog Read Latency

- [ ] `p1` - **ID**: `cpt-metric-cat-nfr-read-latency`

`GET /catalog/metrics` p95 response time **MUST** be ≤ 100ms on a cache hit and ≤ 500ms on a cache miss for a tenant with up to 200 catalog rows.

**Threshold**: p95 ≤ 100ms (hit), ≤ 500ms (miss), measured at the analytics-api service level.

#### Consistency — Write-After-Read Visibility

- [ ] `p1` - **ID**: `cpt-metric-cat-nfr-write-visibility`

A successful threshold write **MUST** be visible to subsequent `GET /catalog/metrics` calls from the same tenant within one cache TTL, with no manual invalidation required.

**Threshold**: 99th-percentile propagation ≤ default cache TTL + 5s buffer.

#### Consistency — Cross-Replica Invalidation

- [ ] `p1` - **ID**: `cpt-metric-cat-nfr-cross-replica-invalidation`

When an admin write on one analytics-api replica invalidates the cache for a tenant, the invalidation **MUST** be observable across all replicas within the NFR threshold. Pure in-process caching (where each replica holds its own LRU and does not coordinate with peers) **MUST NOT** be used; DESIGN picks between a shared cache (e.g., Redis with a tenant-keyed namespace) and a pub-sub broadcast that invalidates peer replicas' in-process caches.

**Threshold**: 99th-percentile cross-replica invalidation latency ≤ 2 seconds after the admin write commits.

**Rationale**: With multi-replica analytics-api, in-process caching would silently fail the write-visibility promise in `cpt-metric-cat-nfr-write-visibility` — a user hitting a different replica from the one that served their admin write would see stale data for up to a full TTL. For a compliance-facing product where admin writes are auditable actions, that staleness is a support-call generator and a regulatory red flag. Committing to a cross-replica mechanism in v1 forces DESIGN to pick a concrete path (Redis vs pub-sub) rather than kicking the problem to production.

### 6.2 NFR Exclusions

- **Accessibility** (UX-PRD-002): Not applicable — catalog is a backend service with no direct UI surface.
- **Internationalization** (UX-PRD-003): The catalog stores `*_i18n_key` references; actually delivering localized copy is outside scope and belongs to the i18n program.
- **Multi-region** (OPS-PRD-005): Not applicable — catalog is per-tenant and tenants are single-region today.
- **Offline support** (UX-PRD-006): Not applicable — consumers are online services.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### GET /catalog/metrics

- [ ] `p1` - **ID**: `cpt-metric-cat-interface-read`

**Type**: REST API

**Stability**: stable

**Description**: Returns the catalog for the caller's tenant, filtered to `is_enabled = true`, with resolved thresholds per the precedence rule in `cpt-metric-cat-fr-scoped-thresholds`. Query parameters (optional): `role_slug`, `team_id` — participate in threshold resolution when supplied. Response shape: `{ tenant_id, generated_at, metrics: [{ metric_key, label_i18n_key, sublabel_i18n_key, description_i18n_key, unit, format, higher_is_better, is_member_scale, source_tags: [string], thresholds: { good, warn, alert_trigger?, alert_bad?, resolved_from } }] }`. `source_tags` is always an array; single-source metrics return a one-element array. The `resolved_from` string identifies which scope supplied the threshold row (v1 domain: `"team+role"`, `"team"`, `"role"`, `"tenant"`, `"product-default"`).

**Breaking Change Policy**: Adding optional fields is non-breaking. Renaming or removing fields requires a major version bump and a two-minor-version deprecation window. When follow-on work adds calculation-rule or query-linkage fields, they **SHOULD** arrive as additive optional fields so v1 consumers keep working unchanged. When the tenant-custom follow-on lifts the v1 `tenant_id IS NULL` CHECK, the response continues to return a single flat `metrics[]` array per tenant (product-owned global rows merged with the caller's tenant-custom rows, keyed by `metric_key`); consumers **MUST NOT** need to distinguish origin to render correctly, and **MAY** additionally read an optional `owner: "product" | "tenant"` field (planned as additive) for UI affordances like "custom" badges.

#### POST/PUT/DELETE /v1/admin/metric-thresholds

- [ ] `p1` - **ID**: `cpt-metric-cat-interface-admin`

**Type**: REST API

**Stability**: stable

**Description**: Admin CRUD for `metric_threshold` rows. Authorization enforced per `cpt-metric-cat-fr-threshold-crud`. Payload validates against `metric_catalog` (`metric_key` must exist and be `is_enabled = true`) and against sanity bounds tied to `higher_is_better`.

**Breaking Change Policy**: Field additions non-breaking; removal is a major bump.

### 7.2 External Integration Contracts

#### Catalog Consumer Contract

- [ ] `p1` - **ID**: `cpt-metric-cat-contract-consumer`

**Direction**: provided by library

**Protocol/Format**: Consumers **MUST** fetch `GET /catalog/metrics` once per session (or per cache-TTL window), key metadata lookups by `metric_key`, and degrade gracefully when a `metric_key` is absent from the response. Consumers **MUST NOT** hardcode metric metadata that the catalog provides.

**Compatibility**: Additive response fields are non-breaking. Catalog entries that disappear (disabled) should be treated by consumers as absent-metadata, not as errors.

## 8. Use Cases

### UC-001 Admin Tunes a Threshold

**ID**: `cpt-metric-cat-usecase-tune-threshold`

**Actor**: `cpt-metric-cat-actor-tenant-admin`

**Preconditions**: Admin has tenant-admin authorization. Target `metric_key` exists in the catalog and is `is_enabled = true`.

**Main Flow**:

1. Admin opens the (future) admin UI or hits the admin API directly
2. Admin submits a new threshold for a `(tenant_id, metric_key)` pair
3. API validates payload — `metric_key` exists, sanity bounds hold, admin is authorized for the tenant
4. API persists the row and invalidates the cache entry for the tenant
5. Next `GET /catalog/metrics` for the tenant returns the new threshold
6. Dashboards in the tenant re-render bullets with the new color policy on their next load

**Postconditions**: Tenant's threshold is updated; no code deploy was required.

**Alternative Flows**:

- **Validation fails**: Payload rejected with a specific error; no state mutated.
- **Admin lacks authorization**: Request rejected with `403`; audit event logged.

### UC-002 Product Team Adds a New Metric

**ID**: `cpt-metric-cat-usecase-new-metric`

**Actor**: `cpt-metric-cat-actor-product-team`

**Preconditions**: The new metric's `metric_query` is either being added in the same PR or already exists in `analytics.metrics`. i18n keys for the new metric are planned in the i18n loader.

**Main Flow**:

1. Product engineer writes a sea-orm migration that inserts one `metric_catalog` row per tenant with appropriate metadata
2. The same PR updates the consuming frontend / service to reference the new `metric_key` and adds or modifies the ClickHouse `query_ref` row if needed
3. Migration runs at service startup in every environment
4. `GET /catalog/metrics` returns the new entry after cache expiry or invalidation
5. Consumers that know the `metric_key` start rendering / evaluating it

**Postconditions**: The new metric is catalog-backed; no hardcoded metadata exists anywhere on the frontend.

**Alternative Flows**:

- **Metric is sensitive to only some tenants**: The migration inserts only for those tenants; others see no entry.
- **Metric is a replacement for an older one**: The older `metric_key` is set to `is_enabled = false` in the same migration; consumers using it fall back to absent-metadata and degrade gracefully.

### UC-003 Consumer Hydrates the Catalog

**ID**: `cpt-metric-cat-usecase-consumer-hydrate`

**Actor**: `cpt-metric-cat-actor-consumer`

**Preconditions**: Consumer has a valid tenant-scoped auth token.

**Main Flow**:

1. Consumer calls `GET /catalog/metrics`
2. API resolves threshold precedence and returns the catalog
3. Consumer caches the response for at most the TTL
4. Consumer uses `metric_key` as the lookup key for any metric-related rendering or evaluation

**Postconditions**: Consumer has a coherent snapshot of the tenant's catalog for the cache window.

**Alternative Flows**:

- **Catalog empty or all-disabled**: Consumer degrades gracefully (render ComingSoon, skip alert evaluation).
- **API 5xx**: Consumer uses last-good cached copy if any; otherwise surfaces a diagnostic error.

## 9. Acceptance Criteria

- [ ] `metric_catalog` and `metric_threshold` tables exist in analytics-api's MariaDB, with sea-orm migrations that seed the frontend metadata one-to-one
- [ ] `metric_catalog` has a nullable `tenant_id` column with a v1 CHECK constraint enforcing `tenant_id IS NULL`; every v1 row is a global product-owned row and no v1 behavior depends on the column; the column exists so the tenant-custom follow-on lifts the CHECK additively without a breaking migration
- [ ] v1 write paths (seed migrations, any internal row-creation code) never produce a row with `tenant_id IS NOT NULL`; the DB CHECK rejects such a row as a backstop
- [ ] `metric_catalog.source_tags` is an array of strings in the API response (one-element for single-source metrics); storage shape is DESIGN's choice but the API contract does not change
- [ ] `metric_threshold.lock_expires_at` column exists with a CHECK constraint enforcing `NULL` in v1; no v1 behavior depends on it
- [ ] Admin write on replica A makes the change visible to `GET /catalog/metrics` served by replica B within `cpt-metric-cat-nfr-cross-replica-invalidation`'s threshold
- [ ] `metric_threshold.scope` is an enum over `{ product-default, tenant, role, team, team+role }`; `role_slug` and `team_id` are nullable VARCHAR columns without FK constraints in v1
- [ ] `GET /catalog/metrics` accepts optional `role_slug` and `team_id` query parameters and resolves thresholds per the precedence `team+role → team → role → tenant → product-default`
- [ ] `GET /catalog/metrics` surfaces `resolved_from` per metric threshold (value in `{ team+role, team, role, tenant, product-default }`) so consumers can explain which scope supplied each value
- [ ] A `product-default` row exists for every seeded `is_enabled = true` metric, so the resolution chain never returns null for an enabled metric
- [ ] `POST /v1/admin/metric-thresholds` persists a new threshold at any v1 scope and invalidates the tenant's cache; the value is visible to the next `GET /catalog/metrics` for the matching `(tenant, role, team)` context without waiting for TTL
- [ ] An attempt to create a threshold with `warn > good` on a `higher_is_better = true` metric is rejected with a validation error; the mirrored constraint (`warn < good` on `higher_is_better = false`) is also rejected
- [ ] `POST /v1/admin/metric-thresholds` rejects any request with a missing-or-wrong combination of scope fields — e.g., `scope = 'role'` with null `role_slug`, `scope = 'team'` with null `team_id`, `scope = 'team+role'` with either null, `scope = 'tenant'` with a non-null `role_slug` or `team_id`
- [ ] Threshold resolution for a request with both `role_slug` and `team_id` set returns the `team+role` row when one exists, the `team` row when not, the `role` row when not, the `tenant` row when not, and `product-default` as the final floor
- [ ] `metric_threshold.is_locked` defaults to `false` and is persisted alongside each row
- [ ] When a `tenant`-scope row has `is_locked = true`, a resolution call for a request with `role_slug` and `team_id` set returns the locked `tenant` row — not the `role` / `team` / `team+role` rows that would otherwise match — and `resolved_from` surfaces `"tenant"`
- [ ] An attempt to `POST /v1/admin/metric-thresholds` creating a `team` or `role` or `team+role` row for a metric whose `tenant`-scope row is locked is rejected with HTTP 403 and an error body naming `blocking_scope`, `blocking_row_id`, and `locked_at`
- [ ] An attempt to `POST /v1/admin/metric-thresholds` with `scope ∈ { role, team, team+role }` AND `is_locked = true` is rejected — v1 permits locks only on `product-default` and `tenant` rows
- [ ] Setting `is_locked = true` populates `locked_by` (from the authenticated actor), `locked_at` (server timestamp), and `lock_reason` (non-empty string from the caller); clearing the lock resets all three to NULL
- [ ] A lock-set write without `lock_reason` is rejected by the API with HTTP 400; an attempt to insert a row into `metric_threshold` with `is_locked = true` and `lock_reason IS NULL` is rejected by the DB CHECK constraint
- [ ] The follow-on release that introduces FK constraints on `role_slug` / `team_id` also ships `cpt-metric-cat-fr-integrity-check`; the integrity job runs on a schedule and flags orphaned references in admin diagnostics
- [ ] Every rejected write returning `403 threshold_locked` also emits a structured audit event containing `actor_id`, `tenant_id`, `metric_key`, `attempted_scope`, `attempted_values`, `blocking_scope`, `blocking_row_id`, `locked_by`, `locked_at`, and event timestamp, persisted to **both** the analytics-api structured log stream and a dedicated `threshold_lock_audit` MariaDB table
- [ ] A successful lock-set and a successful lock-clear each emit corresponding `lock_set` / `lock_cleared` events to both sinks, so full lifecycle is queryable from the audit table
- [ ] `threshold_lock_audit` table retention is ≥ 1 year; retention is enforced either by partition lifecycle or a scheduled pruning job (exact mechanism is DESIGN's choice)
- [ ] All CHECK constraints declared in `cpt-metric-cat-fr-threshold-storage` (including `is_locked = false OR lock_reason IS NOT NULL` and `lock_expires_at IS NULL`) are enforced at **both** the DB layer (via migration DDL) and the application layer (via CRUD validation with structured error responses)
- [ ] The frontend ships a release that removes `BULLET_DEFS`, `IC_KPI_DEFS`, and the metric-metadata portion of `METRIC_KEYS`, hydrating from `GET /catalog/metrics` instead, with no visible change to end users on day one. A transitional release that keeps the local constants as a fallback is acceptable and **SHOULD** be followed by a release that removes the fallback.
- [ ] The empty `analytics.thresholds` table is explicitly resolved (renamed, extended, or dropped) in the same migration series — this PRD is the canonical owner of that decision
- [ ] Disabling a metric via `is_enabled = false` does not error any consumer that was previously rendering it; the consumer degrades gracefully

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| MariaDB | Hosts `metric_catalog` and `metric_threshold` | p1 |
| Analytics API service | Hosts the read and admin write endpoints | p1 |
| Existing `analytics.metrics` table | Holds `query_ref` rows; remains authoritative for how metrics are computed. Not modified by this PRD. | p2 |
| Frontend i18n loader | Resolves `label_i18n_key`, `sublabel_i18n_key`, `description_i18n_key` values to display strings | p2 (for FE consumers) |
| Auth / RBAC | Authorizes tenant-admin writes against threshold CRUD | p1 |
| Dashboard Configurator PRD (`docs/domain/dashboard-configurator/specs/PRD.md`) | Defines `role_catalog` (per-tenant role taxonomy) and a team-identifier convention. v1 of this catalog does not block on it: `role_slug` / `team_id` are string references without FK. The follow-on migration that adds FK constraints onto `role_catalog` and the future team-catalog is gated on Dashboard Configurator's delivery. | p2 (future) |

## 11. Assumptions

- Tenants are single-region; the catalog does not need multi-region replication.
- i18n keys are resolved by consumers (the frontend i18n loader today; future backend consumers may embed a resolver or display the raw keys in admin contexts).
- The product team is the owner of metric semantics in v1; tenants cannot rename labels or change units without a code change. This is intentional per `cpt-metric-cat-fr-metadata-writes`.
- Auth plumbing (tenant admin detection) exists or will be provided alongside this work; modelling new auth primitives is outside scope. Specifically, this PRD assumes the auth layer exposes at minimum: (a) `actor_id` — stable identifier for the authenticated caller, used in audit events; (b) `tenant_id` — the tenant context of the request; (c) `is_tenant_admin(tenant_id)` — boolean predicate gating writes in `cpt-metric-cat-fr-threshold-crud` and lock-set writes in `cpt-metric-cat-fr-threshold-lock`. Additional capabilities — `can_override_lock(metric_key, blocking_scope)`, `can_bypass_lock_as_compliance_admin` — are modelled in the Auth PRD (not this one); this PRD assumes they exist as predicates the analytics-api can query.
- Cache layer (Redis or equivalent shared cache, or a pub-sub broadcast on top of in-process caches) is available in the analytics-api service stack; pure in-process caching is incompatible with `cpt-metric-cat-nfr-cross-replica-invalidation`.
- **Tenant-custom maintenance contract** (applies when the tenant-custom follow-on ships; moot in v1): the product commits silver-layer backward-compat guarantees only for metrics in the **global** (`tenant_id IS NULL`) catalog. Tenants that add custom metrics (`tenant_id = X`) whose `query_ref` or formula depends on silver-layer columns accept responsibility for updating their custom metrics when those columns change in a product release. Product release notes list silver-schema changes in a machine-readable format so tenants can audit impact. This assumption is what makes silver evolution tractable: append-only / no-destructive-changes applies to fields that the **product-owned** catalog depends on; it does not extend to every field a tenant might have bound to. Without this contract, tenant-custom inflates the "append-only silver forever" obligation from a finite product surface to an unbounded union of tenant choices.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Seed-from-frontend import introduces subtle metadata divergence | UI regressions on day one of rollout | Byte-for-byte comparison between rendered bullet output before and after migration; hold the frontend-delete step until comparison passes; transitional release that keeps local constants as fallback provides a revert-path if divergence slips past the comparison |
| Cache invalidation on admin write is missed across replicas | "I changed the threshold, nothing happened" support calls if only one replica's in-process cache is invalidated | DESIGN picks between shared cache (Redis), pub/sub broadcast invalidation, or accepting TTL-bounded staleness. If TTL-bounded staleness is chosen, `cpt-metric-cat-nfr-write-visibility` is relaxed and documented accordingly. Integration tests exercise the chosen path at every admin write endpoint |
| Existing empty `analytics.thresholds` table left ambiguous causes confusion | Future engineers treat it as the live threshold store and diverge | `cpt-metric-cat-fr-thresholds-table-resolution` forces a migration-time decision |
| Metadata writes open up via admin UI in a later version without discipline | Cross-tenant drift of metric meaning; i18n explosion | `cpt-metric-cat-fr-metadata-writes` prohibits runtime metadata edits in v1; future PRDs that propose relaxing this must address the cross-tenant comparability trade-off explicitly |
| v1 catalog ships as admin-editable MariaDB store, then conflicts with a future silver-plugin-manifest-driven model | Either two sources of truth for metadata, or a disruptive migration to unify them | v1 table shape is deliberately compatible with plugin-declared metadata: thresholds are the only admin-writable content; metadata is migration-only. If silver plugins become the source of truth for metadata, the migration that lands plugin sourcing becomes an additive write-path — rows still land in the same table, just populated by a different agent. See §13 OQ. |
| ~~Per-tenant metadata duplication across `(tenant_id, metric_key)` rows~~ | — | **Resolved in v1.7**: `metric_catalog` keyed globally by `metric_key`; no duplication. |
| Threshold override precedence misunderstood by consumers | Inconsistent bullet colors across surfaces for the same metric | `cpt-metric-cat-fr-scoped-thresholds` fixes precedence; the `GET /catalog/metrics` response always returns `resolved_from` so the winning scope is visible without a diagnostics call |
| Scope explosion if future PRDs keep adding scopes (e.g., `org_unit`, `period`, `seniority`, `dashboard`) | Precedence chain becomes unmaintainable, admins cannot predict which row wins | Resolution precedence is canonical and lives in the catalog. This PRD commits to the set `{ product-default, tenant, role, team, team+role }` and treats new scopes as a major-version bump. `dashboard` scope was explicitly rejected in v1.4 (see Changelog and §4.2). Future scope additions require a DESIGN / ADR that updates the precedence diagram. |
| Team-scoped threshold written by a team lead contradicts a tenant admin's rollout intent | Org-wide comparability fragments silently across teams | The `GET /catalog/metrics` response surfaces `resolved_from`, so tenant admins can audit which teams have overrides and why; governance on team-scoped thresholds (justification, expiry, caps) is tracked as an OQ and revisited once the first team-lead authoring surface lands |
| Role / team string references in `metric_threshold` (no FK in v1) drift from `role_catalog.role_slug` or the future team-catalog | Dangling threshold rows that never resolve for anyone after a role rename or team dissolution | (a) Admin CRUD validates that `role_slug` belongs to the caller's tenant's `role_catalog` *once that table exists* — until then, v1 validates only the enum-shape rules in `cpt-metric-cat-fr-scoped-thresholds`. (b) A periodic integrity check (`cpt-metric-cat-fr-integrity-check`) flags rows whose `role_slug` / `team_id` no longer matches any live taxonomy entry and surfaces them in diagnostics — this FR is gated on `role_catalog` delivery and ships with the follow-on FK-adding migration. (c) The follow-on FK-adding migration runs a pre-flight that reports orphaned rows for the admin to resolve before the FK lands. |
| **v1-only risk: dangling role_slug / team_id before the integrity check ships** | Between v1 launch and `role_catalog` delivery, no automated mechanism catches stale references. If HR renames a role in the upstream source, the catalog's `role` / `team+role` rows silently stop matching any live role for anyone. Support gets "why is my threshold not applied?" tickets. | v1 explicitly accepts this — it is a known cost of shipping role/team scopes without waiting for Dashboard Configurator. Mitigation is admin discipline: when a role in the HR source is renamed or deprecated, the tenant admin is responsible for editing the corresponding `metric_threshold` rows. Document this expectation in the admin runbook that ships alongside v1. Audit: once `role_catalog` lands, `cpt-metric-cat-fr-integrity-check` turns on and surfaces any residue. |
| Auth service delivery slips and blocks the catalog write path | Admin CRUD (`cpt-metric-cat-fr-threshold-crud` and `cpt-metric-cat-fr-threshold-lock`) are unusable without `is_tenant_admin(tenant_id)` predicate from Auth. If the Auth PRD ships weeks later than the catalog's DESIGN / implementation, either the whole release waits or CRUD is shipped with a gaping authorization hole. | Define auth dependency as a Rust trait (`TenantAuthorization`) consumed by the admin-handlers; the catalog's own code compiles and passes tests against a test-double implementation. For staging and local-dev environments, provide a configuration-driven stub (e.g., `AUTH_STUB_TENANT_ADMINS=<csv>` env var) that returns canned results. Production deployment requires the real Auth implementation. This decouples catalog release readiness from Auth release readiness: catalog can ship to staging behind the stub while Auth finalizes, and production deploy gates on real Auth being wired. |
| Team-level threshold silently overrides a role-level company standard (by design: `team` beats `role` in the precedence chain) | A tenant admin who has set a company-wide PM-role standard finds that some teams quietly render with different bars | This is an intentional property of the precedence, not a bug — team leads are accountable for their team's outcomes and their call wins over company role standards. Mitigation is visibility + governance, not precedence change: `resolved_from` on every `GET /catalog/metrics` response tells the admin which scope won; an admin diagnostics view (future) lists all `team`-scope rows diverging from the corresponding `role` or `tenant` defaults; write authorization, justification, expiry, and caps on team-scope deviation are tracked in Open Questions. A tenant that wants no team-level divergence for certain metrics restricts write access to `team` scope at the auth layer. |
| **Tenant-custom metrics break when silver schema evolves** (applies when follow-on ships; no v1 exposure) | Custom `query_ref` or formula depends on a silver-layer column that the product drops, renames, or re-shapes in a release. The tenant's custom metric returns NULL or an error; worst case it silently returns wrong numbers if the column name survives with changed semantics. If the product treated silver as append-only-forever to mitigate this, silver evolution would be permanently frozen — a much larger cost than the one being mitigated. | Explicit assumption in §11 that tenants own maintenance of their custom metrics; product guarantees silver backward-compat only for product-owned catalog. Product releases ship a machine-readable manifest of silver-schema changes so tenants can audit impact. Admin UI (follow-on) surfaces "this custom metric references silver column X that was changed in release Y" diagnostics where feasible. This risk does **not** apply to v1 because no tenant-custom rows can exist (CHECK constraint); it is documented here so the follow-on PRD does not accidentally reopen "append-only-forever" as the default mitigation. |

## 13. Open Questions

| Question | Owner | Target Resolution |
|----------|-------|-------------------|
| ~~Existing empty `analytics.thresholds` table — rename/extend, drop, or keep as a generic store?~~ **Resolved**: drop in the same migration series that creates `metric_threshold` — the legacy table is empty and provides no value to keep. Dashboard Configurator PRD updated accordingly. | Backend tech lead | Closed |
| ~~`metric_catalog` keying — per-tenant or global?~~ **Resolved in v1.7**: global `metric_key` is the primary key; no `tenant_id` column on `metric_catalog`. Per-tenant metadata overrides (if ever needed) land through an additive `metric_catalog_tenant_override` table without touching `metric_catalog`. Per-tenant thresholds continue to live in `metric_threshold`. | Insight Product Team + Backend tech lead | Closed v1.7 |
| Source of truth for `label_i18n_key` strings — are they defined in the backend seed migration and consumed by the frontend i18n loader, or defined in the frontend i18n loader and referenced by the migration? | Insight Product Team + Frontend tech lead | Before the seed migration PR |
| Cache invalidation mechanism (shared cache vs pub/sub broadcast) — NFR `cpt-metric-cat-nfr-cross-replica-invalidation` commits v1 to one of them, but not which. Redis with tenant-keyed namespaces is simpler operationally; pub-sub preserves in-process cache latency advantages at the cost of dual-layer complexity. | Backend tech lead | DESIGN phase |
| Should the catalog support per-tenant metadata localization (tenant admin rewriting `label_i18n_key`), or is the product-team-owned model sufficient long-term? Trade-off: cross-tenant comparability vs tenant-specific terminology. | Insight Product Team | Before the second tenant onboards |
| ~~Alerting consumer contract — will alerting use the same `GET /catalog/metrics` endpoint or a narrower `GET /catalog/metrics?for=alerting` variant?~~ **Resolved**: alerting and visual thresholds share the `metric_threshold` table, distinguished by kind. The alerting subsystem (separate domain) consumes the same table filtered to alerting kinds. If a narrower endpoint is later useful, it lands as a path-based variant (e.g., `/catalog/alerting/metrics`) rather than a query parameter — query strings cache poorly through CDN and gateway layers. | Insight Product Team | Closed |
| Soft-delete / audit log of threshold changes — ship in v1 or defer? Regulated tenants may require it from day one. | Insight Product Team | Before v1 is cut |
| **How-is-this-calculated** surface (deferred out of v1) — where does structured calculation metadata live when it lands: a `metric_calculation` JSON on the catalog row, a dedicated table, a silver-plugin manifest shipped with the plugin that produces the metric, or a dbt lineage view? The choice is downstream of whether the silver-plugin-manifest direction (Roman's proposal) moves forward; landing calc rules inside the catalog now would duplicate whatever plugins ship later. | Insight Product Team + Backend tech lead | Before a follow-on PRD for calculation transparency |
| **Silver-plugin manifests as the catalog's upstream source** — the metric_catalog table is designed to be populated by migration in v1 and potentially by plugin manifests in the future. Do we want a dedicated PRD for the plugin contract before v1 ships, or is the "keep schema plugin-friendly, actual plugin integration later" approach acceptable? | Insight Product Team + Backend tech lead | Before a follow-on catalog revision |
| Conditional / filtered / formula-based thresholds (e.g., "count commits with LOC ≤ 5K" as a threshold rule) — v1 uses numeric scalars only. When and how do we evolve thresholds to support structured conditions? Possibly via a threshold-plugin type in the broader plugin architecture. | Insight Product Team | Before any use case requires conditional thresholds |
| Connector-readiness diagnostics — `source_tag` indicates "this metric is produced from Cursor". If a tenant doesn't have Cursor wired up, the metric renders NULL. Do we expose a per-tenant "missing connectors for these metrics" diagnostic via the catalog, or push that to the Identity / Connector PRDs? | Insight Product Team | Before admin UI PRD kicks off |
| ~~**Tenant-specific custom metrics** — path selection~~ **Resolved in v1.10**: path (a) chosen — nullable `tenant_id` column on `metric_catalog` with v1 CHECK `IS NULL`. v1 ships the schema slot; the follow-on PRD lifts the CHECK, adds CRUD for rows with `tenant_id = :caller`, and updates read resolution to `WHERE tenant_id = :caller OR tenant_id IS NULL`. Path (b) (separate `metric_catalog_tenant_custom` table) rejected — avoids a union on every read without adding meaningful isolation, and symmetry with how thresholds layer overrides on the same table is a clearer mental model. | Insight Product Team + Backend tech lead | Closed v1.10 |
| **Tenant-custom query-layer storage** — when the follow-on lands CRUD for tenant-custom catalog rows, where does the *computation* live? Three candidates, not mutually exclusive: **(i)** tenant-scoped rows in `analytics.metrics` — custom metric references its own `query_ref` UUID; requires making `analytics.metrics` tenant-aware and addressing SQL validation / sandboxing (parser, resource governor, column-level ACL) as a sub-project. **(ii)** formula DSL poverh existing metrics — e.g., `metric_X * 0.5 + metric_Y`, no raw SQL, bounded variability, safer to validate, directly maps to the web-authoring UX the prototype demonstrated. Weaker expressivity (no joins, no new aggregations over silver). **(iii)** both — (ii) as the default tenant-author surface with (i) as an escape hatch for enterprise customers who need full expressivity and accept the validation overhead. v1 catalog schema supports any of the three without change. | Insight Product Team + Backend tech lead | Follow-on PRD for tenant-custom |
| **Disable-for-me slot** — when the follow-on tenant-custom ships, does a tenant need a way to **hide** a product-owned metric for themselves (e.g., "we don't think `cc_active` is correct for us, remove it from our dashboards") without forking / editing the product metric? Candidate extension: additive table `metric_catalog_tenant_disable (tenant_id, metric_key)` with a matching read-path filter. Simple, preserves product ownership of the metric, gives tenant the escape hatch without an edit surface. Not required in v1; not required in the first tenant-custom release either, but worth designing alongside so it is not a third migration. | Insight Product Team | When first tenant asks, or alongside first tenant-custom release |
| **`metric_catalog` PK evolution strategy** — v1 behavior is "one row per `metric_key` (all global)". Two PK choices with identical v1 observable behavior but different follow-on migration cost: **(α)** `PK = metric_key` in v1; when tenant-custom lifts the CHECK, PK is rebuilt to `(tenant_id, metric_key)` with NULL-global as a distinct key. Simpler v1 schema, one-time migration pain later (MariaDB PK rebuild on a growing table). **(β)** `PK = id BIGINT AUTO_INCREMENT` + `UNIQUE(tenant_id, metric_key)` from v1 (relying on MariaDB's treatment of multiple NULLs as distinct in unique indexes, which the spec allows but is worth a pre-flight DESIGN confirmation). Slightly more complex v1, no PK rebuild in the follow-on. | Backend tech lead | DESIGN phase |
| Multi-source metrics storage — API contract is fixed as `source_tags: string[]` per v1.7. Storage shape is open: (a) JSON array column on `metric_catalog` — simple, no joins, but no referential integrity and awkward indexing; (b) dedicated `metric_source` join table `(metric_key, source_tag)` — normalized, supports queries like "all metrics depending on Cursor", adds one join to every read; (c) hybrid — JSON array for reads, join table for diagnostics. | Backend tech lead | DESIGN phase |
| Team-scope governance — given that `team` beats `role` by design, what controls do tenant admins get over team-lead writes: (a) RBAC restricting who can write `team` scope, (b) required justification field on team-scope writes, (c) automatic expiry / review window, (d) caps on how far a team-scope value may diverge from the corresponding `role` or `tenant` value, or (e) none of the above, rely on `resolved_from` visibility plus soft governance via admin diagnostics? | Insight Product Team | Before the first team-lead threshold authoring surface ships |
| Role / team reference timing — v1 ships `role_slug` / `team_id` as string columns without FK. When does the FK-adding follow-on migration land, and does it run against `role_catalog` (owned by Dashboard Configurator) or does the metric-catalog grow its own `role_catalog` and team-catalog to stay self-sufficient? | Insight Product Team + Backend tech lead | Before team-lead threshold authoring surface ships |
| **Lock governance** — who is authorized to set `is_locked = true` on a row at each scope? Candidate model: tenant-admin for `tenant` scope locks; product-team (via seed migration only) for `product-default` locks; narrower-scope locks reserved for the same authority as the scope itself. Open question whether a separate "compliance-admin" role is needed that can lock but not edit other thresholds. | Insight Product Team | Before `cpt-metric-cat-fr-threshold-lock` ships |
| ~~Lock audit-event destination — logs-only vs dedicated table?~~ **Resolved in v1.8**: both. Structured logs for real-time observability, dedicated `threshold_lock_audit` MariaDB table for long-term audit retention and admin export. Committed in v1 so audit retention doesn't depend on log-aggregation retention policy. | Insight Product Team + Backend tech lead | Closed v1.8 |
| Lock audit-event retention — the `threshold_lock_audit` table ships with ≥ 1 year retention by default, but regulatory cycles vary (banking: 7 years, healthcare: similar, general SOC2: often 1 year). Per-tenant retention configuration, or a global policy? How is retention enforced — manual partition pruning, a scheduled job, MariaDB partition lifecycle? | Insight Product Team + Backend tech lead | Before first regulated tenant onboards |
| Lock granularity — v1 locks are atomic at the row level (`is_locked = true` pins `good`, `warn`, `alert_trigger`, `alert_bad` together). Is there a use case for per-field locking (e.g., pin `good` as a compliance floor but allow team leads to tighten `warn`)? v1 default is atomic; revisit if a concrete use case shows up. | Insight Product Team | Revisit on demand |
| Temporal locks — `lock_expires_at` column ships in v1 with a CHECK constraint forcing NULL. When the FR for temporal locks lands, what are the semantics on expiry: (a) the lock row auto-deletes, (b) `is_locked` auto-flips to `false` but row stays, (c) the lock remains active but a background job raises an alert in admin diagnostics? Also: is expiry advisory (does nothing by itself) or enforced (background job clears stale locks)? | Insight Product Team + Backend tech lead | Before temporal-lock FR ships |
| **Lock expiry / review** — should locks auto-expire after N days and require re-confirmation, or stay indefinite until explicitly cleared? Indefinite is simpler; time-boxed forces periodic review, which matches compliance audit cycles but requires a background job. | Insight Product Team | Before regulated tenant onboards |
| ~~`product-default` locks — can the product team ship a metric with a locked `product-default` row?~~ **Resolved in v1.7**: yes, but only through seed migrations (not at runtime), and **no runtime super-admin override exists**. Legitimate legal-override requests flow through backend code migrations that either lift the lock or remove the product-default row for the specific tenant. **v1.8 nuance**: the deployment pipeline for such "compliance-emergency" migrations MAY skip non-essential gates (canary, non-blocking lint) to compress hours-to-days turnaround down to ~1 hour while preserving the git-auditable review and deploy trail. Runtime feature-flag bypass is explicitly rejected — it is a super-admin override under a different name and erodes the same compliance guarantee. | Insight Product Team | Closed v1.7, nuance added v1.8 |

## 14. Current-State Gap Analysis (Backend)

This section is informative, not normative. It captures the delta between today's backend code on cyberfabric/insight and what this PRD requires, so DESIGN can scope work accurately.

### Present

- `services/analytics-api` is a running Rust / Axum service with a working `/v1/metrics`, `/v1/metrics/{id}/query`, `/v1/thresholds`, `/v1/persons/{email}`, `/v1/columns` surface. SeaORM entities cover `metrics`, `thresholds`, `table_columns`. Sea-orm migrations run at service startup.
- `analytics.metrics` table holds `query_ref` rows keyed by UUID; one `query_ref` can emit multiple `metric_key` values. Untouched by this PRD.
- `analytics.thresholds` table exists but is empty; its fate is explicitly resolved in this PRD's migration series per `cpt-metric-cat-fr-thresholds-table-resolution`.
- Frontend `src/screensets/insight/api/thresholdConfig.ts` defines `BULLET_DEFS` and `IC_KPI_DEFS`; `src/screensets/insight/types/index.ts` defines `METRIC_KEYS`. These are the canonical sources of current metadata and the seed input for `cpt-metric-cat-fr-seed-from-frontend`.

### Missing — v1

- No `metric_catalog` or `metric_threshold` tables, nor their SeaORM entities.
- No `/v1/catalog/metrics` or `/v1/admin/metric-thresholds` endpoints.
- No cache layer in analytics-api dedicated to catalog reads; the shape of the layer (in-process vs shared) is a DESIGN question.
- No seed migration that imports current frontend metadata.
- RBAC for admin-scope endpoints (tenant-admin only) is not modelled today; `auth.rs` authenticates but does not carry a role claim that threshold CRUD can gate on.

### Missing — Follow-on

- No `role_catalog` table — owned by the Dashboard Configurator PRD (PR #226, branch `docs/dashboard-configurator-prd`). v1 of this catalog does not block on it: `role_slug` is a string column. The follow-on FK-adding migration is gated on `role_catalog` delivery.
- No team-catalog table — no PRD currently owns it. v1 of this catalog ships `team_id` as a string column, matching whatever convention Identity Resolution exposes at the time of writing.
- No calculation-rule storage, invariant-test framework, rule↔SQL coupling gate, `primary_query_id` linkage, admin-diagnostics endpoint, or reverse-lookup endpoint — deferred as a follow-on, likely authored alongside whichever direction wins for calculation transparency (catalog-side structured rules vs silver-plugin manifests vs dbt lineage).
- No silver-plugin manifest concept in the codebase today; the catalog is designed to stay plugin-friendly but does not presume plugins.
- No admin surface for authoring team-scope or role-scope thresholds — this lands when the admin UI PRD is written. Until then, writes go through the API directly or through seed migrations.
- No tenant-custom metrics CRUD — v1 ships the `metric_catalog.tenant_id` column slot with CHECK `IS NULL` but no endpoints, no validation, no authoring surface. The follow-on PRD owns: (a) lifting the CHECK, (b) admin CRUD for `tenant_id = :caller` rows, (c) query-layer decision per §13 OQ, (d) silver-schema change-notification mechanism per §11 Assumption and §12 Risk, (e) optional disable-for-me table per §13 OQ. Not gated on Dashboard Configurator; gated on a product decision that the first enterprise custom-metric customer is worth the validation-surface investment.

### Relevant Open Pull Requests

| PR | Relationship to this PRD |
|----|--------------------------|
| **#226 — docs(dashboard-configurator): PRD** (dzarlax) | Sibling PRD; defines `role_catalog` (per-tenant role taxonomy) and the team-identifier convention. v1 of this catalog ships role / team scope with string refs; the FK-adding follow-on migration is gated on this PRD's delivery. |
| **#214 — MariaDB persons store + migration runner** (Gregory91G) | Orthogonal; establishes the pattern for service-owned MariaDB migrations that this catalog follows (ADR-0006). |
| **#223 — gold views honest-null contract** (merged) | Unblocks the "no `null%` / `nullh` in rendered DOM" story for consumers of `GET /catalog/metrics`. |

### Implementation Sequencing (informative)

A plausible order, each step shippable in isolation:

**Constraint enforcement — both layers**: v1 relies on multiple MariaDB CHECK constraints — `metric_catalog.tenant_id IS NULL`, `metric_threshold.is_locked = false OR lock_reason IS NOT NULL`, `metric_threshold.lock_expires_at IS NULL`, and the `scope`-enum / `role_slug` / `team_id` combinations. DESIGN must ship these as **both** (a) enforced DDL via the SeaORM migration on the target MariaDB version (10.2+ enforces CHECK by default; older silently parse-and-ignore), and (b) **application-layer validation in the admin CRUD path** with explicit structured error mapping per constraint. Belt-and-suspenders is deliberate: DB CHECK is the ultimate backstop against writes that bypass the API (direct SQL, migrations, legacy import paths); app-layer validation owns the user-facing error messages and guarantees correct behavior even if the target MariaDB version silently drops a CHECK or the SeaORM version has a known bug around CHECK emission. Do not ship app-layer-only or DB-only; ship both.

1. Resolve `analytics.thresholds` table fate in DESIGN (pick one of rename/extend, drop, or keep-as-generic-store).
2. Analytics-api: add `metric_catalog` + `metric_threshold` tables with v1 scope domain `{ product-default, tenant, role, team, team+role }` and the v1 seed migration importing FE metadata. `role_slug` / `team_id` are string columns without FK. `metric_catalog` carries a nullable `tenant_id` column with CHECK enforcing `NULL`, reserving the tenant-custom extension path additively; PK-evolution choice (α keep `metric_key` PK, rebuild later; β synthetic `id` + `UNIQUE(tenant_id, metric_key)` from day one) decided in DESIGN.
3. Analytics-api: add `GET /catalog/metrics` (with cache, optional `role_slug` / `team_id` query params, and `resolved_from`) plus `POST/PUT/DELETE /v1/admin/metric-thresholds` accepting all v1 scopes.
4. Frontend: hydrate from `GET /catalog/metrics` alongside existing local metadata as fallback; ship a release that introduces this dual-path and passes the current viewer's `(role, team)` as query params where known.
5. Frontend: remove the local metadata fallback in the immediately following release, once byte-for-byte output comparison confirms parity.
6. (Follow-on, gated on Dashboard Configurator delivering `role_catalog` + a team-catalog decision) FK-add migration that constrains `role_slug` → `role_catalog.role_slug` and `team_id` → whatever table wins as the team source of truth. Includes a pre-flight orphan-row report for admins. Ships together with `cpt-metric-cat-fr-integrity-check` — the scheduled background job that surfaces newly-orphaned references after the migration lands (e.g., after a future role rename).
7. (Follow-on, independent of Dashboard Configurator) Author the calculation-transparency follow-on PRD once the silver-plugin-manifest direction is decided — direction (catalog structured rules vs silver-plugin manifests vs dbt lineage) decided in its own scope.
