> [!WARNING]
> **Under review — audited against the implementation and found inaccurate in places.**
> Read it against the code, not as authority. The specific claims the code contradicts
> are listed in the repository [README](../../../../../../README.md#backend-specs--under-review). Where this
> document and the committed `openapi.json` disagree, the contract is right.

# Technical Design — Identity

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
  - [3.8 Schema + Naming Conventions](#38-schema--naming-conventions)
- [4. Additional context](#4-additional-context)
  - [4.1 Configuration surface](#41-configuration-surface)
  - [4.2 Logging shape](#42-logging-shape)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

`insight-identity-resolution` is a synchronous read path over the
multi-source observation log in MariaDB `persons`. It collapses
observations into a single `PersonResponse` per request by ranking rows
per `(insight_source_type, insight_source_id, value_type)` partition and
picking the latest value per `value_type` across sources. The service
is stateless beyond its connection pool, owns its database (SeaORM
migrations applied via the service's `migrate` subcommand), and follows
the layered `api` → `domain` → `infra` module split of the gears-rust
host (ported from the retired .NET service, epic #1602).

The vision is **operational simplicity**: zero in-memory cache, every
read hits MariaDB on a covered index; first-install behaviour is
"every lookup returns 404" rather than "crash loop"; logs are
PII-redacted JSON; failures are RFC 7807 problem-details bodies with
a sanitised `db_target` for DB exceptions and a stable error URN.

### 1.2 Architecture Drivers

Architecture-shaping decisions are captured as ADRs in
[`ADR/`](ADR/):

- [`cpt-insightspec-adr-0002-read-from-mariadb-persons`](ADR/0002-read-from-mariadb-persons.md) — Read From the MariaDB `persons` Table.
- [`cpt-insightspec-adr-0003-latest-per-source-semantics`](ADR/0003-latest-per-source-semantics.md) — Latest-Per-Source Lookup Semantics.
- [`cpt-insightspec-adr-0004-lowercase-email-lookup`](ADR/0004-lowercase-email-lookup.md) — Lowercase Emails on Storage and Lookup (**Superseded by ADR-0011**).
- [`cpt-insightspec-adr-0005-tenant-context-strategy`](ADR/0005-tenant-context-strategy.md) — Composite Tenant Context With JWT Stub.
- [`cpt-insightspec-adr-0006-display-name-split-fallback`](ADR/0006-display-name-split-fallback.md) — Display-Name Split Fallback.
- [`cpt-insightspec-adr-0007-value-type-routing`](ADR/0007-value-type-routing.md) — `value_type` Routing.
- [`cpt-insightspec-adr-0008-bamboohr-identity-inputs-extension`](ADR/0008-bamboohr-identity-inputs-extension.md) — Extend BambooHR `identity_inputs`.
- [`cpt-insightspec-adr-0009-post-profile-with-uniqueness-invariant`](ADR/0009-post-profile-with-uniqueness-invariant.md) — `POST /v1/profiles` with single-result invariant (Phase 2).
- [`cpt-insightspec-adr-0010-org-chart-cache`](ADR/0010-org-chart-cache.md) — Materialised SCD2 cache for person parent/child edges (`org_chart`).
- [`cpt-insightspec-adr-0011-persons-relax-uniqueness-and-collation`](ADR/0011-persons-relax-uniqueness-and-collation.md) — Persons relax UNIQUE + switch `value_id` to case-insensitive collation.
- [`cpt-insightspec-adr-0012-admin-only-orgchart-visibility-reads`](ADR/0012-admin-only-orgchart-visibility-reads.md) — Admin-only reads on `/v1/visibility`, `/v1/roles`, `/v1/person-roles`.
- [`cpt-insightspec-adr-0015-self-scoped-visibility-read-without-admin`](ADR/0015-self-scoped-visibility-read-without-admin.md) — `POST /v1/visible-persons` answers the caller's own visible set without the admin gate.
- [`cpt-insightspec-adr-0013-roles-hard-delete-with-in-use-guard`](ADR/0013-roles-hard-delete-with-in-use-guard.md) — `roles` hard-DELETE guarded by active-assignment count (422 `urn:insight:error:role_in_use`).
- [`cpt-insightspec-adr-0014-last-admin-protection`](ADR/0014-last-admin-protection.md) — Refuse to revoke the last active admin assignment in a tenant.

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| [`cpt-insightspec-fr-identity-lookup-resolve-by-email`](PRD.md#resolve-email-to-person_id) | `persons_repo::resolve_person_ids_by_email` issues a `SELECT person_id FROM persons WHERE value_type='email' AND value_id=? AND insight_tenant_id=? ORDER BY created_at DESC LIMIT 1` against the `idx_value_id` covered index. |
| `cpt-insightspec-fr-identity-lookup-resolve-by-person-id` | `value_type='person_id'` needs no resolution step: `api/handlers.rs::resolve_person_id_mode` validates the UUID (nil and non-UUID are 400s, never a silent empty resolution), rejects the source fields (a person id is tenant-wide), and confirms the person exists in the tenant via `persons_repo::person_exists`; an unobserved id yields no candidate so the handler answers 404 like an unknown email. Visibility applies unchanged, so a person's name and their metrics answer to ONE permission — the SPA routes on `person_id` since the identity cutover, and a person without a current email is reachable only this way. |
| [`cpt-insightspec-fr-identity-lookup-hydrate`](PRD.md#hydrate-person-attributes) | `persons_repo::fetch_person_observations` runs a `ROW_NUMBER() OVER (PARTITION BY ...)` CTE returning one row per (source, value_type); the assembler in `domain/profile.rs` then picks the latest across sources. |
| [`cpt-insightspec-fr-identity-lookup-404`](PRD.md#not-found-returns-rfc-7807) | The profile handler returns an RFC 7807 body with `type=urn:insight:error:person_not_found`, `status=404` when the resolve step matches nothing. |
| [`cpt-insightspec-fr-identity-lookup-400-tenant`](PRD.md#missing-tenant-returns-rfc-7807) | The tenant comes from the verified JWT's `tenant_id` claim (oidc-authn-plugin SecurityContext); when no tenant resolves the handler returns an RFC 7807 body (`type=urn:insight:error:tenant_unresolved`, `status=400`). |
| [`cpt-insightspec-fr-identity-lookup-parent`](PRD.md#surface-parent-attributes-when-present) | `handlers::resolve_parent` reads the single CURRENT parent edge from `org_chart` filtered to the configured `org_chart_source_type` (BambooHR by default — Phase 2 of #348), then hydrates the parent's own observations to fill `supervisor_email` (parent's email), `supervisor_name` (parent's display name), and the legacy `parent_*` triple (`parent_email` = parent's email, `parent_id` = parent's `value_type='id'` on the same source instance, `parent_person_id` = the edge's `parent_person_id`). The `ParentProjection` flows into the profile assembler; stale `value_type='parent_*'` observations in `persons` are ignored. |
| [`cpt-insightspec-fr-identity-lookup-subordinates`](PRD.md#recursively-expand-subordinates) | `handlers::resolve_subordinates` walks `persons_repo::current_children_for_parent` recursively via `hydrate_children`, filters edges to the configured `org_chart_source_type`, hydrates each child with the same recursion (depth-counted), and feeds the resulting list into the response's `subordinates`. Cycle protection is a visited-set of `person_id`s; depth cap reads from the gear config's `max_depth`. The same recursion serves `/v1/profiles`. |
| [`cpt-insightspec-fr-identity-routing-name-split`](PRD.md#display-name-split-fallback) | The display-name split fallback runs after assembly when both `first_name` and `last_name` observations are missing. |
| [`cpt-insightspec-fr-identity-migrations-startup`](PRD.md#service-owned-migrations-at-startup) | The service's `migrate` subcommand runs the SeaORM migrator (`src/migration/`, SQL embedded from `src/migration/sql/`) before the service serves traffic; in Kubernetes it runs as an initContainer. |
| [`cpt-insightspec-fr-identity-schema-relax-uniqueness`](PRD.md#schema-allows-recording-state-transitions) | Migration `004_persons_relax_constraints.sql` drops `UNIQUE uq_person_observation` on `(..., value_hash)` and adds the same name on `(..., created_at)`. The seeder's `INSERT IGNORE` in step 7 now dedupes by `created_at` (re-runs idempotent) while genuine transitions on the same partition (Active->Inactive->Active) persist as separate rows. ADR-0011 documents the design decision. |
| [`cpt-insightspec-fr-identity-schema-case-insensitive-value-id`](PRD.md#value-comparisons-are-case-insensitive) | The same migration `ALTER COLUMN value_id MODIFY ... COLLATE utf8mb4_unicode_ci`. `idx_value_id` rebuilds under the new collation; existing SQL (`WHERE value_id = @x`) is now case-insensitive without code changes. `value_full_text` is already `utf8mb4_unicode_ci`; `value` (TEXT) uses table default `utf8mb4_unicode_ci`; `value_hash` (CHAR ascii) stays `ascii_bin` as it is a SHA-256 digest. |
| [`cpt-insightspec-fr-identity-profile-resolve`](PRD.md#resolve-profile-by-email-or-source-native-id) | The `POST /v1/profiles` handler (`api/handlers.rs::resolve_profile`) routes by the request's `value_type` (`email` or `id`) to `persons_repo::resolve_person_ids_by_email` / `resolve_person_ids_by_source_id`. Both queries are CTEs with partition `(insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type)` and `rn=1` filter — the canonical latest-per-source-instance projection. |
| `cpt-insightspec-fr-identity-visible-persons-batch` | `POST /v1/visible-persons` (`api/visible_persons.rs::filter_visible_persons`) answers which of the requested canonical person ids (UUIDs) the caller may see. `subchart_repo::visible_targets` materialises the visible-set union once — caller, active grants, the whole tenant on a wildcard grant, `org_chart` descendants — and joins the requested ids against it; `has_wildcard_grant` short-circuits the traversal (echoing the request, so its answer is a subset of the input rather than a tenant-existence check). No email resolution step exists: the metrics runtime keys on `person_id` since the identity cutover, so the ids arrive canonical. Roles are absent from the predicate, so the `admin` role confers no visibility (ADR-0015). |
| [`cpt-insightspec-fr-identity-profile-ambiguous-422`](PRD.md#surface-single-result-invariant-via-422) | The resolve step distinguishes found / not-found / ambiguous. When the reader returns `>1` distinct `person_id`, the handler emits an RFC 7807 extension body carrying the lookup body + the matched `person_ids` list with status 422. |
| [`cpt-insightspec-fr-identity-profile-ids-list`](PRD.md#project-full-alias-list-on-response) | `persons_repo::current_source_ids_for_person` returns the latest `value_type='id'` per source instance; the profile assembler ships the list unchanged into the `ProfileResponse` wire shape (`domain/profile.rs::ProfileIdEntry`). |
| [`cpt-insightspec-fr-identity-profile-org-tree`](PRD.md#project-the-same-org-tree-shape-as-v1persons) | The profile handler hydrates the same `Person` tree (`hydrate_person`) that the retired GET endpoint returned, copying `supervisor_email` / `supervisor_name` / `parent_email` / `parent_id` / `parent_person_id` / `subordinates` straight off the projection. Identical `Person` shape across callers — guaranteed by reusing the recursion. |
| [`cpt-insightspec-fr-identity-profile-validation`](PRD.md#validate-request-body-via-fluentvalidation) | Request-body validation in the handler expresses the cross-field rules (`value_type=='id'` requires source coordinates; `value_type=='email'` forbids them) before resolving the tenant; first-error wins on `urn:insight:error:*` URN. (Ported from the retired .NET FluentValidation validator.) |
| [`cpt-insightspec-fr-identity-org-chart-table`](PRD.md#materialised-parentchild-edge-cache) | Migration `003_org_chart.sql` adds the SCD2 edge table with PK `(tenant, source_type, source_id, child, valid_from)`, CHECK `no_self_loop`, and indexes on current-parent / current-children / cross-source views; ADR-0010 records the design decision. |
| [`cpt-insightspec-fr-identity-org-chart-rebuild`](PRD.md#rebuild-edges-from-persons-deterministically) | `seed-persons-from-identity-input.py` step 9 builds `org_chart_next` from a UNION of `value_type='parent_person_id'` (Source 1, future reconciliation) and `value_type='parent_email'` JOINed to the latest `value_type='email'` observation per `(tenant, value_id)` partition (Source 2, current pipeline); Source 1 wins via NOT EXISTS guard. Step 5 sorts accounts BambooHR-first so the canonical `supervisorEmail` source establishes `person_id`s before downstream connectors. Source 2 intersects each `parent_email` period with the child's **active intervals** derived from `value_type='status'` observations (Active/Inactive/Terminated, with LAG to collapse duplicates and LEAD to compute interval ends); children without any status observation get a synthetic [-inf,+inf) interval. Re-activation (Inactive -> Active) produces a fresh row rather than reopening the closed one — SCD2 history is preserved. Two-table swap via `RENAME` mirrors step 8. Parent_emails with no email-bearer in `persons` are skipped and counted in the seeder log (no stubs created — see ADR-0010). Post-swap two-hop cycle detection self-joins CURRENT edges and emits a WARN line if `(A->B)` and `(B->A)` co-exist; deeper cycles are bounded structurally by the Phase-3 subchart endpoint's depth parameter. |
| [`cpt-insightspec-fr-identity-org-chart-read`](PRD.md#read-current-parent-and-children-edges) | `persons_repo::current_parents_for_child` / `current_children_for_parent` issue `SELECT ... WHERE child_person_id=? AND valid_to IS NULL` (respectively `parent_person_id=?`); the query strings live with the repository, materialised as `OrgChartEdge` rows. |

#### NFR Allocation

| Requirement | Design Response |
|-------------|-----------------|
| [`cpt-insightspec-nfr-identity-latency`](PRD.md#p95-lookup-latency) | Single-row covered-index lookup (`idx_value_id`) + the SeaORM (SQLx) connection pool; pool max size tuned to 16 (smaller than the analytics service per design review). |
| [`cpt-insightspec-nfr-identity-memory`](PRD.md#memory-budget-without-caching) | No in-memory cache; helm `resources.limits.memory: 384Mi`; query results materialised row-by-row from the driver. |
| [`cpt-insightspec-nfr-identity-logging-pii`](PRD.md#structured-json-logs-with-pii-redaction) | Structured JSON logs via the gears host's tracing subscriber; email-bearing path segments are logged as route templates, never raw values; the error layer emits a sanitised `db_target` for DB errors only. |
| [`cpt-insightspec-nfr-identity-uuid-roundtrip`](PRD.md#binary16-uuid-round-trip) | All UUID parameters are bound as 16-byte `BINARY(16)` values (big-endian RFC 4122 order) and read back the same way. Integration test pins the round-trip. |

### 1.3 Architecture Layers

| Layer | Responsibility | Module |
|-------|----------------|--------|
| **Api** | HTTP surface — route handlers, request/response DTOs, auth (JWT-derived tenant), error → RFC 7807 mapping. | `src/api/` |
| **Domain** | Lookup orchestration + observation collapse — profile assembly, display-name split fallback, subchart tree building, seed logic. Pure Rust, no DB or HTTP types. | `src/domain/` |
| **Infrastructure** | Persistence + migrations — SeaORM pool, `persons_repo` and sibling repositories, centralised named SQL (`sql_named.rs`), the SeaORM migrator + `src/migration/sql/*.sql`. | `src/infra/` |

Dependency direction is strict: api → domain → infra; domain
does not reference SeaORM or HTTP framework types. Repository
functions in `infra/db/` materialise rows into the domain shapes.

## 2. Principles & Constraints

### 2.1 Design Principles

#### Observation log, not relational tree

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-observation-log`

The reader treats `persons` as an append-only event log. There are no
foreign-key joins for org-tree traversal — the supervisor edge is
expressed as `parent_person_id` observations written by the
reconciliation service. Phase 1 surfaces those observations
verbatim; Phase 2 will walk them recursively. The service never
mutates `persons` — that is the seed pipeline's and the future
reconciliation service's job.

#### Centralised SQL

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-centralised-sql`

SQL statements live with the repositories under
`src/infra/db/` (named statements centralised in `sql_named.rs`).
A schema evolution (column rename, index addition) touches one place;
the repository is purely binding + materialisation. This keeps the
"how" of the latest-per-source CTE auditable in one place.

#### Tenant from the verified JWT, config default opt-in

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-tenant-composite`

The tenant is the `tenant_id` claim of the gateway JWT verified by the
oidc-authn-plugin (mapped to `subject_tenant_id` in the
SecurityContext). The config `tenant_default_id` is an opt-in default
for single-tenant clusters (and the bootstrap-admin seed).
Multi-tenant production overlays leave the default empty. (The .NET
service's header-first composite resolver is retired — a tenant from
the outside world never passes.)

#### Fail fast at startup, not at first request

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-fail-fast`

The SeaORM migrator (the `migrate` subcommand, run as an initContainer
in Kubernetes) completes before the service serves traffic. A bad
connection string or a failed migration fails the pod immediately;
kubelet retries. The service never serves traffic against an
unmigrated database.

#### PII boundary at the logger

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-pii-boundary`

Every log enrichment that touches the request goes through an
allow-list. Email-bearing path segments are logged as route templates,
never raw values. There is no log line outside the structured tracing
framework — no `println!`, no raw email interpolation.

### 2.2 Constraints

#### gears-rust host / workspace Rust toolchain

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-dotnet-9`

The service is a gear on the gears-rust host and builds with the
workspace-pinned Rust toolchain (ported from the retired .NET 9
implementation, epic #1602). It ships in the shared backend workspace
at `src/backend/services/identity-resolution/`.

#### SeaORM MySQL backend for MariaDB

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-mysqlconnector`

The MariaDB-flavoured wire protocol is served by SeaORM's MySQL
(SQLx) backend. The dependency is pinned in the service's
`Cargo.toml` and surfaced via the `infra/db` module; no other code
path in domain or api touches the driver.

#### SeaORM migrator for migrations

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-dbup-version`

The SeaORM migrator is the migration mechanism (see ADR-0006; it
replaced the retired .NET service's DbUp). Migration steps live in
`src/migration/` (one Rust file per step) and embed their SQL from
`src/migration/sql/`. They are applied via the service's `migrate`
subcommand, which also runs the migrate-time first-admin bootstrap.

#### `BINARY(16)` for every UUID

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-binary16-uuid`

A byte-exact UUID round-trip is required (NFR-uuid-roundtrip). No
column may store a UUID as a 36-char `CHAR(36)` — the schema, the
parameter binding, and the read path all enforce 16-byte bytes.

#### Structured JSON logging only

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-serilog-compact-json`

No console plain-text logging is allowed in production builds.
A local-dev config overlay may enable human-readable console output,
but production stays structured JSON for log aggregation parity.

## 3. Technical Architecture

### 3.1 Domain Model

| Concept | Representation | Notes |
|---------|---------------|-------|
| `Person` | `domain::profile::PersonResponse` (immutable). Fields: `person_id`, `email`, `display_name`, `first_name`, `last_name`, `department`, `division`, `job_title`, `status`, `supervisor_email`, `supervisor_name`, `parent_email`, `parent_id`, `parent_person_id`, `subordinates`. The `supervisor_*` pair and the legacy `parent_*` triple are both populated from the single `org_chart` edge filtered to the configured `org_chart_source_type`; `subordinates` is the recursive BambooHR-only subtree (empty list = leaf). | Wire shape — see [`PRD.md#get-v1personsemail--person-lookup`](PRD.md#get-v1personsemail--person-lookup). |
| `Profile` | `domain::profile::ProfileResponse` — superset of `Person` for `POST /v1/profiles`. Adds `insight_tenant_id`, `username`, `employee_id`, and `ids[]` (all current `value_type='id'` observations, one per source instance). Optional fields are nullable rather than empty strings; the API layer drops nulls from JSON. | Wire shape — see [`PRD.md#post-v1profiles--profile-resolution`](PRD.md#post-v1profiles--profile-resolution). |
| `PersonObservation` | One row from `persons` projected into `(insight_source_type, insight_source_id, value_type, value_effective, created_at)` by `infra/db/persons_repo.rs`. | Domain-level shape; `value_effective` is the DB-generated coalesce. |
| `OrgChartEdge` | One CURRENT parent->child edge from `org_chart` (`infra/db/persons_repo.rs::OrgChartEdge`). Fields: `insight_source_type`, `insight_source_id`, `child_person_id`, `parent_person_id`, `valid_from`. | Domain-level; not part of the wire surface. |
| `ParentProjection` | `domain::profile::ParentProjection` — the parent edge resolved into the fields the assembler writes: parent's `person_id`, `email`, `display_name`, and source-native id (on the same source instance as the edge). | Internal contract between parent resolution (producer) and profile assembly (consumer). |
| `PersonSourceId` | One source-native id binding for the `ids[]` projection on the profile response. Fields: `insight_source_type`, `insight_source_id`, `value`. | Domain-level shape; wire form is `ProfileIdEntry`. |
| `ValueTypes` | Constants enumerating canonical `value_type` strings. | Free-form on the DB side; the enumeration documents the set the assembler projects. |
| Persons reader | Repository functions in `infra/db/persons_repo.rs` — `resolve_person_ids_by_email` / `fetch_person_observations` (lookup), `resolve_person_ids_by_source_id` / `current_source_ids_for_person` (profile resolution), `current_parents_for_child` / `current_children_for_parent` (org_chart reads). | The infra layer's read surface over `persons` / `org_chart`. |
| Tenant context | The verified JWT's `tenant_id` claim, mapped by the oidc-authn-plugin into the SecurityContext (`subject_tenant_id`). | Config `tenant_default_id` is an opt-in default for single-tenant clusters. |
| `LookupOptions` | Lookup options passed from the API layer into hydration: `expand_subordinates`, `max_depth`, `org_chart_source_type`. Parent hydration is unconditional (always populated when an `org_chart` edge exists); only the subordinates recursion is gated. | Bound from the gear config (`GearConfig`) per request. Defaults: `expand_subordinates: true`, `max_depth: 16`, `org_chart_source_type: "bamboohr"`. |

### 3.2 Component Model

#### api module (`src/api/`)

- [ ] `p1` - **ID**: `cpt-insightspec-component-identity-api`

##### Why this component exists

To translate HTTP requests into domain calls and domain results into
RFC 7807 responses, owning every concern that is HTTP- or
hosting-specific so that domain and infra remain free of HTTP
framework types.

##### Responsibility scope

- Registers the route handlers on the gears-rust host's router.
- Consumes configuration from the gear config
  (`gears.identity-resolution.config` in the host YAML) +
  `APP__gears__identity-resolution__config__*` env overrides.
- Wires the SeaORM pool, the repositories, and the seed worker into
  the handler state.
- Structured JSON logging via the host's tracing subscriber
  (`service=identity-resolution`, PII-redacting request logging).
- Migrations are applied by the service's `migrate` subcommand
  (initContainer in Kubernetes) before the serving process starts.
- Maps `POST /v1/profiles` (the successor of the retired
  `GET /v1/persons/{email}` — dropped together with the .NET service,
  zero callers), `GET /internal/persons/by-email/{email}`
  (internal-only), `/health`, `/healthz`.
- Implements the error mapping that emits RFC 7807
  bodies with sanitised `db_target` for DB errors only.

##### Responsibility boundaries

- Does **not** issue SQL. Repository access is via the `infra/db`
  repositories only.
- Does **not** parse `persons` rows. Materialisation is in
  `persons_repo`.
- Does **not** apply migrations at request time — the SeaORM migrator
  runs in the `migrate` subcommand.

##### Related components (by ID)

- `cpt-insightspec-component-identity-domain` — orchestrates lookups.
- `cpt-insightspec-component-identity-infra` — persistence + migrations.
- `cpt-insightspec-actor-api-gateway` — sole external caller in Phase 1.

#### domain module (`src/domain/`)

- [ ] `p1` - **ID**: `cpt-insightspec-component-identity-domain`

##### Why this component exists

To carry the lookup orchestration and observation-collapse logic in
a layer that has zero compile-time coupling to the HTTP framework or
SeaORM. This keeps unit tests of the assembly and display-name-split
logic fast and makes the algorithm legible in isolation from the
SQL strings.

##### Responsibility scope

- Email lookup — trims the email, resolves `person_id`
  (case-insensitive via the column collation per ADR-0011), fetches
  latest-per-source observations, hands them to the assembler.
- Profile assembly (`domain/profile.rs`) — collapses per-`value_type`
  observations across sources by latest `created_at`, falls back to
  the display-name split when `first_name`/`last_name` are absent.
- Display-name split — handles `"Last, First"`
  and `"First Last"` formats; single-token names yield
  `(token, "")`.
- Canonical `value_type` constants used by the assembler.
- Subchart tree building (`domain/subchart.rs`) and the persons-seed
  logic (`domain/seed.rs`, `domain/seed_service.rs`).

##### Responsibility boundaries

- Does **not** open MariaDB connections — that's the SeaORM pool in
  infra.
- Does **not** know which `value_type` routes to which physical
  column — that's the seed pipeline's contract (ADR-0007) and the
  repository's SQL.
- Does **not** map results to JSON — that's the api layer's
  serialiser.

##### Related components (by ID)

- `cpt-insightspec-component-identity-api` — consumes the lookup
  service.
- `cpt-insightspec-component-identity-infra` — implements
  `IPersonsReader`.

#### infra module (`src/infra/`)

- [ ] `p1` - **ID**: `cpt-insightspec-component-identity-infra`

##### Why this component exists

To isolate every MariaDB-specific detail (connection handling,
`BINARY(16)` parameter binding, `ROW_NUMBER()` CTE, the SeaORM
migrator) in one module so the domain code stays portable and so a
future read replica or backup target can be swapped in without
touching the lookup algorithm.

##### Responsibility scope

- SeaORM connection pool over the configured `database_url`
  (`mysql://user:pass@host:port/db`); log context uses a sanitised
  target (`host:port/db`, no creds).
- `persons_repo` and sibling repositories (`roles_repo`,
  `person_roles_repo`, `visibility_repo`, `subchart_repo`,
  `seed_repo`, `ops_repo`) — bind UUIDs as `BINARY(16)` bytes and
  materialise domain rows.
- `sql_named.rs` — centralised named SQL; the latest-per-source CTE
  is one of the documented SQL artefacts (see §3.7).
- The SeaORM migrator (`src/migration/`) — one Rust step per change,
  SQL embedded from `src/migration/sql/`, run by the `migrate`
  subcommand together with the first-admin bootstrap
  (`infra/db/bootstrap.rs`).
- The ClickHouse `identity_inputs` reader
  (`infra/identity_inputs.rs`) over the shared HTTP ClickHouse client
  (port 8123).

##### Responsibility boundaries

- Does **not** decide tenant routing or display-name fallback —
  that's Domain.
- Does **not** emit HTTP responses — that's Api.
- Does **not** orchestrate the seed pipeline — that's
  `src/backend/services/identity-resolution/seed/`.

##### Related components (by ID)

- `cpt-insightspec-component-identity-domain` — implements its
  port.
- `cpt-insightspec-actor-mariadb` — runtime target.

### 3.3 API Contracts

This section enumerates the public interfaces declared in the PRD's
Public Library Interfaces section (§7) and pins them to concrete
implementation details.

| PRD Interface | Implementation | Notes |
|---------------|----------------|-------|
| [`cpt-insightspec-interface-identity-person-lookup`](PRD.md#get-v1personsemail--person-lookup) | **Retired** together with the .NET service (approved removal, zero callers). `POST /v1/profiles` (`api/handlers.rs::resolve_profile`) is the successor; an internal-only `GET /internal/persons/by-email/{email}` remains for in-cluster use. Snake-case JSON. | Kept for historical traceability. |
| [`cpt-insightspec-interface-identity-health`](PRD.md#get-health--database-readiness) | Health handler — opens a connection, runs `SELECT 1`. | 200 / 503. |
| [`cpt-insightspec-interface-identity-healthz`](PRD.md#get-healthz--process-liveness) | Liveness handler returning `"ok"`. | Never touches DB. |

External contracts:

- [`cpt-insightspec-contract-identity-env-config`](PRD.md#identity_-env-var-contract) —
  honoured by the gears-rust host's config loader: YAML section
  `gears.identity-resolution.config` with
  `APP__gears__identity-resolution__config__<field>` env overrides;
  bound to the strongly-typed `GearConfig` struct.
- [`cpt-insightspec-contract-identity-config-secret`](PRD.md#insight-identity-config-secret) —
  consumed via `envFrom: secretRef: insight-identity-resolution-config`
  in the Deployment template (see
  `src/backend/services/identity-resolution/helm/`).

### 3.4 Internal Dependencies

| Dependency Module | Interface Used | Purpose |
|-------------------|----------------|---------|
| `src/domain/` | Profile assembly, display-name split, subchart + seed logic | Lookup orchestration + observation collapse. |
| `src/infra/` | `persons_repo` + sibling repositories, SeaORM pool, SeaORM migrator | MariaDB persistence + migrations. |
| `charts/insight/templates/secrets.yaml` (umbrella) | Emits `insight-identity-resolution-config` with `APP__gears__identity-resolution__config__database_url` etc. | Runtime config supply. |
| `charts/insight/templates/mariadb-initdb-scripts.yaml` (umbrella) | Provisions empty `identity` database + grants on first MariaDB pod boot. | Empty DB substrate for the SeaORM migrator. |

### 3.5 External Dependencies

| Dependency | Version | Why | Failure mode |
|------------|---------|-----|--------------|
| sea-orm (MySQL/SQLx backend) | workspace-pinned | MariaDB-flavoured wire protocol; `BINARY(16)` UUID binding; connection pool. | Pool exhaustion → 503 on `/health`; pod restart on transient connectivity loss. |
| sea-orm-migration | workspace-pinned | Schema migration applied via the `migrate` subcommand; tracks `seaql_migrations`. | Failed migration → the `migrate` subcommand exits non-zero, the initContainer fails and the pod never serves. |
| tracing (gears-rust host subscriber) | workspace-pinned | Structured JSON logs, request logging, PII redaction. | Logger init failure → process exits; no fallback. |
| insight-clickhouse (HTTP client) | workspace | Reads `identity.identity_inputs` over ClickHouse HTTP (port 8123) for the persons-seed worker. | Seed run fails with a bounded timeout; the read API is unaffected. |
| testcontainers (Rust, test only) | workspace-pinned | Spins up a real MariaDB for integration tests (`cargo test -p identity-resolution`). | Test failure when Docker unavailable; not a runtime concern. |

### 3.6 Interactions & Sequences

#### Person lookup happy path

- [ ] `p1` - **ID**: `cpt-insightspec-seq-identity-lookup-happy`

```
api-gateway  →  identity-resolution  →  oidc-authn-plugin (tenant from JWT)
                                                              │
                                                              ▼
                              persons_repo::resolve_person_ids_by_email
                                                              │
                                              (covered idx_value_id)
                                                              ▼
                              persons_repo::fetch_person_observations
                                                              │
                                          (ROW_NUMBER OVER PARTITION)
                                                              ▼
                                              profile assembly (domain)
                                                              │
                                                              ▼
                                                  ProfileResponse (JSON)
                                                              │
                                                              ▼
                                                       api-gateway merges
```

1. api-gateway calls `POST /v1/profiles`
   (`{"value_type":"email","value":"alice@example.com"}`) with the
   ES256 gateway JWT.
2. The oidc-authn-plugin verifies the JWT and maps its `tenant_id`
   claim into the SecurityContext.
3. The handler trims the email (case handled at the storage layer per
   ADR-0011).
4. `persons_repo::resolve_person_ids_by_email` issues
   `SELECT person_id FROM persons WHERE insight_tenant_id=? AND
   value_type='email' AND value_id=? ORDER BY created_at DESC,
   id DESC LIMIT 1` on the `idx_value_id` covered index.
5. `persons_repo::fetch_person_observations` runs the
   `ROW_NUMBER()` CTE, returning one row per (source, value_type).
6. The profile assembler collapses across sources by latest
   `created_at`, running the display-name split if first/last absent.
7. The handler serialises to snake-case JSON; returns 200.

#### Tenant unresolved

- [ ] `p1` - **ID**: `cpt-insightspec-seq-identity-tenant-unresolved`

```
caller  →  identity-resolution  →  tenant resolution
                                       │
                            (no tenant_id claim, no default)
                                       │
                                       ▼
                            RFC 7807 problem body
                                       │
                                       ▼
                       400 + RFC 7807 problem-details
```

The tenant comes from the verified JWT's `tenant_id` claim, with the
config `tenant_default_id` as an opt-in fallback; if neither yields a
tenant, the endpoint returns
`urn:insight:error:tenant_unresolved` with status 400.

#### Startup with migration

- [ ] `p1` - **ID**: `cpt-insightspec-seq-identity-startup`

```
kubelet  →  initContainer: identity-resolution migrate
                                    │
                                    ▼
                          gear config bind (GearConfig)
                                    │
                                    ▼
                          SeaORM Migrator::up
                            │      (steps in src/migration/,
                            │       SQL from src/migration/sql/)
                            ▼
                          first-admin bootstrap (infra/db/bootstrap.rs)
                            │      (failure → non-zero exit, pod restart)
                            ▼
                          main container: serve
                                    │
                                    ▼
                          /health, /healthz, /v1/profiles
```

SeaORM's `seaql_migrations` table guarantees each step applies once
across pod restarts; idempotency is at the script level (every DDL
uses `CREATE TABLE IF NOT EXISTS`). When `bootstrap_admin_person_id`
is configured, the `migrate` subcommand also seeds the first active
`admin` assignment in `tenant_default_id` unless one already exists
(the migrate-time first-admin bootstrap, ported from the .NET
`BootstrapAdminRunner`).

### 3.7 Database schemas & tables

The service is a **reader** of `persons` and the migrator of the
`identity` MariaDB database.

#### Table: `persons` (MariaDB)

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-identity-persons`

Defined in `src/migration/sql/001_persons.sql`
(applied by the SeaORM migrator via the `migrate` subcommand).
Canonical column reference:
[docs/domain/identity-resolution/specs/DESIGN.md §"Table: persons"](../../../../../domain/identity-resolution/specs/DESIGN.md#table-persons-mariadb).

The service reads it via two queries (centralised with the
repository in `src/infra/db/`):

```sql
-- Sql.ResolvePersonIdByEmail
SELECT person_id
FROM persons
WHERE insight_tenant_id = @tenant_id
  AND value_type = 'email'
  AND value_id   = @email
ORDER BY created_at DESC, id DESC
LIMIT 1;

-- Sql.LatestObservationsByPersonId
WITH ranked AS (
  SELECT
    person_id, insight_source_type, insight_source_id,
    value_type, value_effective, created_at,
    ROW_NUMBER() OVER (
      PARTITION BY insight_source_type, insight_source_id, value_type
      ORDER BY created_at DESC, id DESC
    ) AS rn
  FROM persons
  WHERE insight_tenant_id = @tenant_id
    AND person_id         = @person_id
)
SELECT person_id, insight_source_type, insight_source_id,
       value_type, value_effective, created_at
FROM ranked
WHERE rn = 1;
```

Both queries are tenant-scoped first; the `idx_value_id` covered
index satisfies the resolve query without a heap read, and the
`(insight_tenant_id, person_id, ...)` selectivity keeps the
hydrate CTE bounded by per-person observation count (typically
< 100 rows).

#### Table: `account_person_map` (MariaDB)

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-identity-account-person-map`

Defined in `src/migration/sql/002_account_person_map.sql`.
The service migrates the table but does **not** read it in
Phase 1 — the seed pipeline rebuilds it as an SCD2 cache from
`persons` (see
[domain DESIGN §"Table: account_person_map"](../../../../../domain/identity-resolution/specs/DESIGN.md#table-account_person_map-mariadb)).
Future Phase 2 lookups will use it for "as-of" account → person
binding queries.

#### Table: `org_chart` (MariaDB)

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-identity-org-chart`

Defined in `src/migration/sql/003_org_chart.sql`
(see ADR-0010). The service migrates and reads the table — the
seed pipeline (`seed-persons-from-identity-input.py` step 9)
rebuilds it as an SCD2 cache of direct parent->child edges
derived from `persons` via two sources: `value_type='parent_person_id'`
observations (Source 1, future reconciliation service) and
`value_type='parent_email'` observations resolved by JOIN to the
latest matching `value_type='email'` observation per tenant
(Source 2, the current pipeline's only edge producer). Source 2
intersects each parent_email period with the child's active
intervals derived from `value_type='status'` observations.

The Phase-1 invariant is at most one CURRENT edge per
`(tenant, source_type, source_id, child)`; multi-parent (matrix
orgs) becomes a Phase-1.5 change that adds `parent_person_id` to
the PK.

Read paths in Phase 1:
- `persons_repo::current_parents_for_child(tenant, child)` —
  `WHERE child_person_id=? AND valid_to IS NULL` against
  `idx_current_parent`.
- `persons_repo::current_children_for_parent(tenant, parent)` —
  `WHERE parent_person_id=? AND valid_to IS NULL` against
  `idx_current_children`.

Phase 2 callers project these onto the `/v1/persons` and
`/v1/profiles` response shapes (designated-source supervisor +
per-source detail). Phase 3 (`/v1/subchart/{person_id}?depth=N`,
issue #348) walks the table via a depth-bounded recursive CTE in a
single round-trip — `SqlSubchart.GetSubchart` joins a recursive
`subtree` CTE rooted at `@root_person_id` (depth-bounded by the
optional `@max_depth` parameter; null = unbounded, capped by
MariaDB's `cte_max_recursion_depth = 1000`) against a derived
`latest_obs` CTE that picks the latest `(person, value_type)`
observation per partition via `ROW_NUMBER() OVER (...)`. Tenant +
source-type scoping is bound on both CTEs. The result is a flat
row set ordered by depth; the service layer (`domain/subchart.rs`)
assembles the tree by indexing on `parent_person_id`. Visibility is
applied to the root only — the
visibility CTE is closed under `org_chart` descent, so once the
viewer can see the root every descendant is already in their
visible set.

#### Table: `seaql_migrations` (MariaDB, SeaORM-managed)

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-identity-schema-versions`

SeaORM's migration tracker table (it replaced the retired .NET
service's DbUp `SchemaVersions`). Created automatically on the first
`migrate` run if absent; the service does not interact with it
directly. Provides idempotency for pod restarts.

### 3.8 Schema + Naming Conventions

- [ ] `p1` - **ID**: `cpt-insightspec-design-identity-conventions`

Conventions locked during PR #517 for the Identity Resolution
service. Newly added tables and Domain types MUST follow them; an
existing table that violates a convention is migrated at the next
touch (per the project-wide "consistency over scope" rule).

#### SQL columns

1. **Audit datetime columns** use `DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6))`.
   `UTC_TIMESTAMP` forces UTC regardless of session `time_zone`;
   `CURRENT_TIMESTAMP` would defer to session config and silently
   drift under multi-region deploys. MariaDB 10.2+ requires
   expression DEFAULTs to be parenthesised — `DEFAULT (UTC_TIMESTAMP(6))`,
   not `DEFAULT UTC_TIMESTAMP(6)`. `TIMESTAMP` as a column type is
   not used in new tables; it auto-converts to session timezone
   on read.

2. **`valid_from` / `valid_to` SCD2 columns** are `DATETIME(6) NOT NULL`
   (no column-level default — filled at INSERT). `valid_to` is NULL
   for the currently-active row; the schema enforces the sane
   interval order via `CHECK (valid_to IS NULL OR valid_from <= valid_to)`.

   **`valid_from` default-on-INSERT semantics:** when the caller
   doesn't supply `valid_from` (POST body field omitted / null), the
   INSERT statement substitutes `UTC_TIMESTAMP(6)` server-side via
   `IFNULL(@valid_from, UTC_TIMESTAMP(6))`. The repository binds
   SQL NULL for the parameter rather than passing the client
   clock's now; both timestamps in a subsequent
   `valid_to = UTC_TIMESTAMP(6)` soft-delete then come from the same
   clock (the DB server) and the CHECK constraint cannot fail under
   client/server clock skew.

3. **Optional free-text columns** (`reason`, comment-style audit) are
   `VARCHAR(N) NULL` or `TEXT NULL`. NULL == "no value provided",
   distinct from explicit `''`. Existing tables that historically
   used `NOT NULL DEFAULT ''` are migrated to NULL at the next
   touch.

4. **Boolean existence probes** use `SELECT EXISTS (SELECT 1 FROM …)`,
   not `SELECT 1 … LIMIT 1` + null-check. EXISTS returns 0/1
   (never NULL), short-circuits at the planner, and reads as a
   boolean scalar.

5. **BINARY(16) UUID round-trip** uses big-endian RFC 4122 wire
   order: the UUID's canonical bytes (`uuid::Uuid::as_bytes`) when
   binding, and the same order when reading. Established
   by the persons table (see ADR-0002) and mirrored everywhere.

#### Domain entity naming

The Domain record/entity name is the **singular** form of the
table name. Examples:

| Table | Domain entity |
|---|---|
| `visibility` | `Visibility` |
| `roles` | `Role` |
| `person_roles` | `PersonRole` (junction over `persons` + `roles`; mirrors ALMS3 `UserRole`) |

**Exception — observation log / append-only event tables:** when
the table is a log/event-stream AND there's a separate aggregate
that consolidates many rows, the row-entity carries an explicit
suffix to disambiguate. Examples:

| Table | Row entity | Aggregate |
|---|---|---|
| `persons` (observation log per ADR-0003) | `PersonObservation` | `Person` |
| `org_chart` (SCD2 cache of parent→child edges) | `OrgChartEdge` | n/a |

If a future table is unambiguously an entity table (one row =
one identity), use the strict rule. If it's an event log + has a
separate aggregate concept, use the suffix exception.

## 4. Additional context

### 4.1 Configuration surface

Configuration is the gears-rust host config: YAML section
`gears.identity-resolution.config` in `config/insight.yaml`, with
`APP__gears__identity-resolution__config__<field>` env overrides
(bound to `GearConfig`). The listener address lives in the host's
`api-gateway` gear config (`bind_addr: 0.0.0.0:8082` — the same port
the retired .NET service used, so consumers only flipped hostname).

| Config field (env override `APP__gears__identity-resolution__config__<field>`) | Default | Notes |
|---------|---------|-------|
| `database_url` | _none_ (required) | `mysql://user:pass@host:port/db`; percent-encoding allowed for users / passwords. |
| `org_chart_source_type` | `bamboohr` | Source instance whose `org_chart` edges populate supervisor/parent fields. |
| `roster_source_type` | _empty_ | The one source trusted to say who exists. The persons-seed mints a person for its accounts even with no address to match on, stamping `roster-mint` on the binding; each such account reaches the review queue as `minted_from_roster` until an operator confirms or reassigns it, and the person stays out of the merge picker until then. Empty disables the branch — an addressless account then stays unresolved. Requirements a roster must meet: it emits an `id` observation per account (the binding row), and it can report deactivation — a source that cannot say a member left will keep leavers minted and queued. Enabling it is not undone by disabling it: `persons` is append-only, so the minted persons and their queue items remain. Naming a second source is unsupported, and naming a source type that spans several connector instances defeats it the same way — one addressless human listed twice becomes two persons that nothing can join; the seed warns when it sees either. |
| `expand_subordinates` | `true` | Recursive subordinates subtree on profile responses. |
| `max_depth` | `16` | Max org-tree recursion depth (cycle-safe). |
| `clickhouse_url`, `clickhouse_database`, `clickhouse_user`, `clickhouse_password` | `""` / `identity` / `""` / `""` | ClickHouse HTTP coordinates (port 8123) for reading `identity_inputs` (persons-seed input). |
| `tenant_default_id` | _empty_ | Optional; opt-in for single-tenant clusters and the bootstrap-admin seed. |
| `bootstrap_admin_person_id` | _empty_ | Optional; migrate-time first-admin bootstrap. |

### 4.2 Logging shape

Every log line is structured JSON via the gears-rust host's tracing
subscriber with:

- an RFC 3339 timestamp and level.
- the request method, route template, status code, and elapsed time
  on request-logging lines.
- W3C trace and span IDs (when present).
- `service` — `identity-resolution`.
- the route template, never the raw email path segment.
- For unhandled errors: the error chain; `db_target` (sanitised
  `host:port/db`, no creds) is attached only when the error is a
  database error.

## 5. Traceability

| PRD ID | DESIGN reference |
|--------|------------------|
| `cpt-insightspec-fr-identity-lookup-resolve-by-email` | §1.2 Functional Drivers; §3.7 SQL `ResolvePersonIdByEmail`. |
| `cpt-insightspec-fr-identity-lookup-hydrate` | §1.2 Functional Drivers; §3.7 SQL `LatestObservationsByPersonId`. |
| `cpt-insightspec-fr-identity-lookup-404` | §1.2 Functional Drivers; §3.3 API Contracts. |
| `cpt-insightspec-fr-identity-lookup-400-tenant` | §1.2 Functional Drivers; §3.6 Sequence "Tenant unresolved". |
| `cpt-insightspec-fr-identity-lookup-parent` | §1.2 Functional Drivers; `handlers::resolve_parent` + profile assembly. |
| `cpt-insightspec-fr-identity-lookup-subordinates` | §1.2 Functional Drivers; `handlers::resolve_subordinates` recursion + `persons_repo::current_children_for_parent`. |
| `cpt-insightspec-fr-identity-routing-name-split` | §1.2 Functional Drivers; §3.2 domain display-name split. |
| `cpt-insightspec-fr-identity-migrations-startup` | §1.2 Functional Drivers; §3.6 Sequence "Startup with migration". |
| `cpt-insightspec-fr-identity-schema-relax-uniqueness` | §1.2 Functional Drivers; ADR-0011 §Decision Outcome (new UNIQUE on `created_at`). |
| `cpt-insightspec-fr-identity-schema-case-insensitive-value-id` | §1.2 Functional Drivers; ADR-0011 §Decision Outcome (collation switch to `utf8mb4_unicode_ci`). |
| `cpt-insightspec-fr-identity-org-chart-table` | §1.2 Functional Drivers; §3.7 Table `org_chart`; ADR-0010. |
| `cpt-insightspec-fr-identity-org-chart-rebuild` | §1.2 Functional Drivers; rebuild step in seeder (`seed-persons-from-identity-input.py` step 9). |
| `cpt-insightspec-fr-identity-org-chart-read` | §1.2 Functional Drivers; §3.7 read paths note; `persons_repo::current_parents_for_child` / `current_children_for_parent`. |
| `cpt-insightspec-fr-identity-profile-org-tree` | §1.2 Functional Drivers; `handlers::resolve_profile` → `hydrate_person` → profile assembly. |
| `cpt-insightspec-nfr-identity-latency` | §1.2 NFR Allocation; §3.7 covered index. |
| `cpt-insightspec-nfr-identity-memory` | §1.2 NFR Allocation; §2.1 Principle "Observation log, not relational tree". |
| `cpt-insightspec-nfr-identity-logging-pii` | §1.2 NFR Allocation; §4.2 Logging shape. |
| `cpt-insightspec-nfr-identity-uuid-roundtrip` | §1.2 NFR Allocation; §2.2 Constraint "BINARY(16) for every UUID". |
| `cpt-insightspec-interface-identity-person-lookup` | §3.3 API Contracts. |
| `cpt-insightspec-interface-identity-health` | §3.3 API Contracts. |
| `cpt-insightspec-interface-identity-healthz` | §3.3 API Contracts. |
| `cpt-insightspec-contract-identity-env-config` | §3.3 API Contracts; §4.1 Configuration surface. |
| `cpt-insightspec-contract-identity-config-secret` | §3.3 API Contracts; §3.4 Internal Dependencies. |
