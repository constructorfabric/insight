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

`insight-identity` is a synchronous read path over the multi-source
observation log in MariaDB `persons`. It collapses observations into a
single `PersonResponse` per request by ranking rows per
`(insight_source_type, insight_source_id, value_type)` partition and
picking the latest value per `value_type` across sources. The service
is stateless beyond its connection pool, owns its database (DbUp
migrations at startup), and follows the three-project Clean
Architecture split (Api → Domain → Infrastructure) common across the
cyberfabric .NET services.

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
- [`cpt-insightspec-adr-0013-roles-hard-delete-with-in-use-guard`](ADR/0013-roles-hard-delete-with-in-use-guard.md) — `roles` hard-DELETE guarded by active-assignment count (422 `urn:insight:error:role_in_use`).
- [`cpt-insightspec-adr-0014-last-admin-protection`](ADR/0014-last-admin-protection.md) — Refuse to revoke the last active admin assignment in a tenant.

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| [`cpt-insightspec-fr-identity-lookup-resolve-by-email`](PRD.md#resolve-email-to-person_id) | `PersonsRepository.ResolvePersonIdByEmailAsync` issues a `SELECT person_id FROM persons WHERE value_type='email' AND value_id=@email AND insight_tenant_id=@t ORDER BY created_at DESC LIMIT 1` against the `idx_value_id` covered index. |
| [`cpt-insightspec-fr-identity-lookup-hydrate`](PRD.md#hydrate-person-attributes) | `PersonsRepository.GetLatestObservationsAsync` runs a `ROW_NUMBER() OVER (PARTITION BY ...)` CTE returning one row per (source, value_type); `PersonAssembler.Assemble` then picks the latest across sources. |
| [`cpt-insightspec-fr-identity-lookup-404`](PRD.md#not-found-returns-rfc-7807) | `PersonsEndpoints.GetByEmail` returns `Results.Problem(...)` with `type=urn:insight:error:person_not_found`, `status=404` when the resolve step returns null. |
| [`cpt-insightspec-fr-identity-lookup-400-tenant`](PRD.md#missing-tenant-returns-rfc-7807) | `CompositeTenantContext.Resolve` returns null when no resolver fires; endpoint converts to `Results.Problem(type=urn:insight:error:tenant_unresolved, status=400)`. |
| [`cpt-insightspec-fr-identity-lookup-parent`](PRD.md#surface-parent-attributes-when-present) | `PersonLookupService.ResolveParentAsync` reads the single CURRENT parent edge from `org_chart` filtered to the configured `OrgChartSourceType` (BambooHR by default — Phase 2 of #348), then hydrates the parent's own observations to fill `supervisor_email` (parent's email), `supervisor_name` (parent's display name), and the legacy `parent_*` triple (`parent_email` = parent's email, `parent_id` = parent's `value_type='id'` on the same source instance, `parent_person_id` = the edge's `parent_person_id`). The `ParentProjection` flows into `PersonAssembler.Assemble`; stale `value_type='parent_*'` observations in `persons` are ignored. |
| [`cpt-insightspec-fr-identity-lookup-subordinates`](PRD.md#recursively-expand-subordinates) | `PersonLookupService.HydrateAsync` walks `IPersonsReader.GetCurrentChildrenAsync` recursively, filters edges to the configured `OrgChartSourceType`, hydrates each child with the same `HydrateAsync` call (depth-counted), and feeds the resulting list into `PersonAssembler.Subordinates`. Cycle protection is a `HashSet<Guid>` of visited `person_id`s; depth cap reads from `AppOptions.MaxSubordinateDepth`. Same recursion serves `/v1/profiles` via `HydrateForProfileAsync`. |
| [`cpt-insightspec-fr-identity-routing-name-split`](PRD.md#display-name-split-fallback) | `DisplayNameSplitter` runs after assembly when both `first_name` and `last_name` observations are missing. |
| [`cpt-insightspec-fr-identity-migrations-startup`](PRD.md#service-owned-migrations-at-startup) | `Program.cs` calls `MigrationRunner.Run` (DbUp + MySql adapter) before `app.RunAsync()`; embedded SQL resources under `Migrations/`. |
| [`cpt-insightspec-fr-identity-schema-relax-uniqueness`](PRD.md#schema-allows-recording-state-transitions) | `Migrations/004_persons_relax_constraints.sql` drops `UNIQUE uq_person_observation` on `(..., value_hash)` and adds the same name on `(..., created_at)`. The seeder's `INSERT IGNORE` in step 7 now dedupes by `created_at` (re-runs idempotent) while genuine transitions on the same partition (Active->Inactive->Active) persist as separate rows. ADR-0011 documents the design decision. |
| [`cpt-insightspec-fr-identity-schema-case-insensitive-value-id`](PRD.md#value-comparisons-are-case-insensitive) | The same migration `ALTER COLUMN value_id MODIFY ... COLLATE utf8mb4_unicode_ci`. `idx_value_id` rebuilds under the new collation; existing SQL (`WHERE value_id = @x`) is now case-insensitive without code changes. `value_full_text` is already `utf8mb4_unicode_ci`; `value` (TEXT) uses table default `utf8mb4_unicode_ci`; `value_hash` (CHAR ascii) stays `ascii_bin` as it is a SHA-256 digest. |
| [`cpt-insightspec-fr-identity-profile-resolve`](PRD.md#resolve-profile-by-email-or-source-native-id) | New `PersonsEndpoints.MapPost("/v1/profiles")` handler dispatches to `ProfileLookupService.ResolveAsync` which routes by `ResolveProfileKind` (`Email` or `SourceId`) to `IPersonsReader.ResolvePersonIdsByEmailAsync` / `ResolvePersonIdsBySourceIdAsync`. Both reader methods execute CTE queries (`SqlProfiles.cs`) with partition `(insight_tenant_id, person_id, insight_source_type, insight_source_id, value_type)` and `rn=1` filter — the canonical latest-per-source-instance projection. |
| [`cpt-insightspec-fr-identity-profile-ambiguous-422`](PRD.md#surface-single-result-invariant-via-422) | `ProfileLookupService.ResolveAsync` returns a tagged `ProfileLookupResult` (`Found` / `NotFound` / `Ambiguous`). When the reader returns `>1` distinct `person_id`, the endpoint emits an `AmbiguousProfileProblemResponse` (RFC 7807 extension carrying the lookup body + the matched `person_ids` list) with status 422. |
| [`cpt-insightspec-fr-identity-profile-ids-list`](PRD.md#project-full-alias-list-on-response) | `IPersonsReader.GetCurrentSourceIdsAsync` runs `SqlProfiles.CurrentSourceIdsForPerson` returning latest `value_type='id'` per source instance; `ProfileAssembler.Assemble` ships the list unchanged into the response shape; `ProfileResponse.From(Profile)` maps the domain record to the wire DTO. |
| [`cpt-insightspec-fr-identity-profile-org-tree`](PRD.md#project-the-same-org-tree-shape-as-v1persons) | `ProfileLookupService.ResolveAsync` calls `PersonLookupService.HydrateForProfileAsync(tenantId, personId, options)` to build the same `Person` tree the GET endpoint returns, then hands it off to `ProfileAssembler.Assemble` which copies `SupervisorEmail` / `SupervisorName` / `ParentEmail` / `ParentId` / `ParentPersonId` / `Subordinates` straight off the projection. Identical `Person` shape for both endpoints — guaranteed by reusing the recursion. |
| [`cpt-insightspec-fr-identity-profile-validation`](PRD.md#validate-request-body-via-fluentvalidation) | `ResolveProfileCommandValidator` (FluentValidation `AbstractValidator<ResolveProfileCommandModel>`) expresses cross-field rules via `When(value_type=='id', ...)` / `When(value_type=='email', ...)`; registered via `AddValidatorsFromAssemblyContaining<…>` in `Program.cs`; endpoint awaits `validator.ValidateAsync` before resolving tenant; first-error wins on `urn:insight:error:*` URN. |
| [`cpt-insightspec-fr-identity-org-chart-table`](PRD.md#materialised-parentchild-edge-cache) | `Migrations/003_org_chart.sql` adds the SCD2 edge table with PK `(tenant, source_type, source_id, child, valid_from)`, CHECK `no_self_loop`, and indexes on current-parent / current-children / cross-source views; ADR-0010 records the design decision. |
| [`cpt-insightspec-fr-identity-org-chart-rebuild`](PRD.md#rebuild-edges-from-persons-deterministically) | `seed-persons-from-identity-input.py` step 9 builds `org_chart_next` from a UNION of `value_type='parent_person_id'` (Source 1, future reconciliation) and `value_type='parent_email'` JOINed to the latest `value_type='email'` observation per `(tenant, value_id)` partition (Source 2, current pipeline); Source 1 wins via NOT EXISTS guard. Step 5 sorts accounts BambooHR-first so the canonical `supervisorEmail` source establishes `person_id`s before downstream connectors. Source 2 intersects each `parent_email` period with the child's **active intervals** derived from `value_type='status'` observations (Active/Inactive/Terminated, with LAG to collapse duplicates and LEAD to compute interval ends); children without any status observation get a synthetic [-inf,+inf) interval. Re-activation (Inactive -> Active) produces a fresh row rather than reopening the closed one — SCD2 history is preserved. Two-table swap via `RENAME` mirrors step 8. Parent_emails with no email-bearer in `persons` are skipped and counted in the seeder log (no stubs created — see ADR-0010). Post-swap two-hop cycle detection self-joins CURRENT edges and emits a WARN line if `(A->B)` and `(B->A)` co-exist; deeper cycles are bounded structurally by the Phase-3 subchart endpoint's depth parameter. |
| [`cpt-insightspec-fr-identity-org-chart-read`](PRD.md#read-current-parent-and-children-edges) | `IPersonsReader.GetCurrentParentsAsync` / `GetCurrentChildrenAsync` issue `SELECT ... WHERE child_person_id=@c AND valid_to IS NULL` (respectively `parent_person_id=@p`); `SqlOrgChart` holds both query strings, `PersonsRepository.ReadEdgesAsync` is the shared row→`OrgChartEdge` reader. |

#### NFR Allocation

| Requirement | Design Response |
|-------------|-----------------|
| [`cpt-insightspec-nfr-identity-latency`](PRD.md#p95-lookup-latency) | Single-row covered-index lookup (`idx_value_id`) + connection pooling via MySqlConnector; pool max size tuned to 16 (smaller than the analytics service per design review). |
| [`cpt-insightspec-nfr-identity-memory`](PRD.md#memory-budget-without-caching) | No in-memory cache; helm `resources.limits.memory: 384Mi`; query results streamed via `DbDataReader`. |
| [`cpt-insightspec-nfr-identity-logging-pii`](PRD.md#structured-json-logs-with-pii-redaction) | Serilog `CompactJsonFormatter`; `UseSerilogRequestLogging` `EnrichDiagnosticContext` callback rewrites `RequestPath` for `/v1/persons/*` to `/v1/persons/<redacted>`; exception handler emits sanitised `db_target` for DB exceptions only. |
| [`cpt-insightspec-nfr-identity-uuid-roundtrip`](PRD.md#binary16-uuid-round-trip) | All `Guid` parameters bound via `MySqlParameter { MySqlDbType = MySqlDbType.Binary, Size = 16, Value = guid.ToByteArray() }`; reads use `reader.GetBytes` → `new Guid(byte[])`. Integration test pins the round-trip. |

### 1.3 Architecture Layers

| Layer | Responsibility | Project |
|-------|----------------|---------|
| **Api** | HTTP surface — minimal-API endpoints, request/response DTOs, auth (tenant context), exception → RFC 7807 mapping, Serilog wiring. | `Insight.Identity.Api` |
| **Domain** | Lookup orchestration + observation collapse — `PersonLookupService`, `PersonAssembler`, `DisplayNameSplitter`, `ValueTypes`, ports (`IPersonsReader`, `ITenantContext`). Pure C#, no DB or HTTP types. | `Insight.Identity.Domain` |
| **Infrastructure** | Persistence + migrations — `MariaDbConnectionFactory`, `PersonsRepository`, `Sql` (centralised CTE), `MigrationRunner` + embedded `Migrations/*.sql`. | `Insight.Identity.Infrastructure` |

Dependency direction is strict: Api → Domain → Infrastructure; Domain
does not reference MySqlConnector or ASP.NET Core. The
`IPersonsReader` port lives in Domain; `PersonsRepository` (in
Infrastructure) implements it and is registered as singleton in DI.

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

Every `SELECT` lives in `Insight.Identity.Infrastructure/MariaDb/Sql.cs`.
A schema evolution (column rename, index addition) touches one file;
the repository is purely binding + materialisation. This keeps the
"how" of the latest-per-source CTE auditable in one place.

#### Composite tenant resolver, header-first

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-tenant-composite`

`CompositeTenantContext` walks `HeaderTenantContext` → `JwtTenantContext`
(stub) → `ConfigTenantContext` and returns the first non-null. Header
always wins — config default is opt-in for single-tenant clusters.
Multi-tenant production overlays leave the default empty.

#### Fail fast at startup, not at first request

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-fail-fast`

DbUp runs before the HTTP listener opens. A bad connection string or
a failed migration crashes the pod immediately; kubelet retries. The
service never serves traffic against an unmigrated database.

#### PII boundary at the logger

- [ ] `p1` - **ID**: `cpt-insightspec-principle-identity-pii-boundary`

Every log enrichment that touches the request goes through an
allow-list. The email path segment is rewritten to `<redacted>` at the
`UseSerilogRequestLogging` diagnostic-context callback. There is no
log line outside the structured framework — no `Console.WriteLine`,
no raw `ILogger.LogInformation("...{email}", email)`.

### 2.2 Constraints

#### .NET 9 / net9.0 target

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-dotnet-9`

The Domain project's value types use record-struct features and
collection expressions that target `net9.0`. Backporting to `net8.0`
is out of scope until the platform-wide runtime moves.

#### MySqlConnector, not Microsoft.Data.SqlClient

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-mysqlconnector`

MariaDB-flavoured wire protocol requires MySqlConnector. The package
is pinned in `Insight.Identity.Infrastructure.csproj` and surfaced via
the `MariaDbConnectionFactory` abstraction; no other code path in
Domain or Api touches it.

#### DbUp 6.x for migrations

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-dbup-version`

DbUp 6.x is the migrator (see ADR-0006). Embedded SQL resources under
`Insight.Identity.Infrastructure/Migrations/` are surfaced via
`WithScriptsEmbeddedInAssembly(... contains ".Migrations." ...)`.
Earlier 5.x lacked the `IUpgradeLog` adapter used by `MigrationRunner`;
6.0.4+ is the floor.

#### `BINARY(16)` for every UUID

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-binary16-uuid`

`Guid.ToByteArray()` round-trip is required (NFR-uuid-roundtrip). No
column may store a UUID as a 36-char `CHAR(36)` — the schema, the
parameter binding, and the read path all enforce 16-byte bytes.

#### Serilog `CompactJsonFormatter` only

- [ ] `p1` - **ID**: `cpt-insightspec-constraint-identity-serilog-compact-json`

No console plain-text logging is allowed in production builds.
Local-dev YAML overlay may enable the Console sink for readability,
but the formatter stays compact-JSON for log aggregation parity.

## 3. Technical Architecture

### 3.1 Domain Model

| Concept | Representation | Notes |
|---------|---------------|-------|
| `Person` | `Insight.Identity.Domain.Person` (immutable record). Fields: `person_id`, `email`, `display_name`, `first_name`, `last_name`, `department`, `division`, `job_title`, `status`, `supervisor_email`, `supervisor_name`, `parent_email`, `parent_id`, `parent_person_id`, `subordinates`. The `supervisor_*` pair and the legacy `parent_*` triple are both populated from the single `org_chart` edge filtered to `AppOptions.OrgChartSourceType`; `subordinates` is the recursive BambooHR-only subtree (empty list = leaf). | Wire shape — see [`PRD.md#get-v1personsemail--person-lookup`](PRD.md#get-v1personsemail--person-lookup). |
| `Profile` | `Insight.Identity.Domain.Profile` — superset of `Person` for `POST /v1/profiles`. Adds `insight_tenant_id`, `username`, `employee_id`, and `ids[]` (all current `value_type='id'` observations, one per source instance). Optional fields are nullable rather than empty strings; the API layer drops nulls from JSON. | Wire shape — see [`PRD.md#post-v1profiles--profile-resolution`](PRD.md#post-v1profiles--profile-resolution). |
| `PersonObservation` | `Insight.Identity.Domain.PersonObservation` — one row from `persons` projected into `(insight_source_type, insight_source_id, value_type, value_effective, created_at)`. | Domain-level shape; `value_effective` is the DB-generated coalesce. |
| `OrgChartEdge` | One CURRENT parent->child edge from `org_chart`. Fields: `insight_source_type`, `insight_source_id`, `child_person_id`, `parent_person_id`, `valid_from`. | Domain-level; not part of the wire surface. |
| `ParentProjection` | `Insight.Identity.Domain.Services.ParentProjection` — the parent edge resolved into the fields the assembler writes: parent's `person_id`, `email`, `display_name`, and source-native id (on the same source instance as the edge). | Internal contract between `PersonLookupService` (producer) and `PersonAssembler` (consumer). |
| `PersonSourceId` | One source-native id binding for the `ids[]` projection on the profile response. Fields: `insight_source_type`, `insight_source_id`, `value`. | Domain-level shape; wire form is `ProfileIdEntry`. |
| `ValueTypes` | Static class enumerating canonical `value_type` strings. | Free-form on the DB side; the enumeration documents the set the assembler projects. |
| `IPersonsReader` | Port — `ResolvePersonIdByEmailAsync` / `GetLatestObservationsAsync` (Phase 1 lookup), `ResolvePersonIdsByEmailAsync` / `ResolvePersonIdsBySourceIdAsync` / `GetCurrentSourceIdsAsync` (profile resolution), `GetCurrentParentsAsync` / `GetCurrentChildrenAsync` (org_chart reads). | Infrastructure provides `PersonsRepository`. |
| `ITenantContext` | Port — `Guid? Resolve(HttpContext)`. | Implementations: `HeaderTenantContext`, `JwtTenantContext` (stub), `ConfigTenantContext`, `CompositeTenantContext`. |
| `LookupOptions` | `Insight.Identity.Domain.Services.LookupOptions` — passed from the API layer into both lookup services. Fields: `ExpandSubordinates`, `MaxDepth`, `OrgChartSourceType`. Parent hydration is unconditional (always populated when an `org_chart` edge exists); only the subordinates recursion is gated. | Bound from `AppOptions` per request via `PersonsEndpoints.BuildLookupOptions`. `LookupOptions.Default` (`ExpandSubordinates: true`, `MaxDepth: 16`, `OrgChartSourceType: "bamboohr"`) is a test-only convenience — production paths never read it. |

### 3.2 Component Model

#### Insight.Identity.Api

- [ ] `p1` - **ID**: `cpt-insightspec-component-identity-api`

##### Why this component exists

To translate HTTP requests into domain calls and domain results into
RFC 7807 responses, owning every concern that is HTTP- or
hosting-specific so that Domain and Infrastructure remain free of
ASP.NET Core types.

##### Responsibility scope

- Hosts the ASP.NET Core minimal-API app + endpoint mapping.
- Parses configuration from `appsettings.yaml` + `IDENTITY__*` env vars.
- Wires DI: `MariaDbConnectionFactory`, `PersonsRepository`,
  `IPersonsReader`, tenant resolvers, `CompositeTenantContext`,
  `PersonLookupService`.
- Configures Serilog (`CompactJsonFormatter`, `service=identity`
  enricher, PII-redacting request-logging callback).
- Runs `MigrationRunner.Run` before opening the listener.
- Maps `/v1/persons/{email}` (**deprecated** — emits RFC 8594 `Deprecation: true` + `Link: </v1/profiles>; rel="successor-version"`; new callers use `POST /v1/profiles`), `POST /v1/profiles`, `/health`, `/healthz`.
- Implements the global exception handler that emits RFC 7807
  bodies with sanitised `db_target` for DB exceptions only.

##### Responsibility boundaries

- Does **not** issue SQL. Repository access is via `IPersonsReader`
  only.
- Does **not** parse `persons` rows. Materialisation is in
  `PersonsRepository`.
- Does **not** apply migrations directly — delegates to
  `MigrationRunner` in Infrastructure.

##### Related components (by ID)

- `cpt-insightspec-component-identity-domain` — orchestrates lookups.
- `cpt-insightspec-component-identity-infra` — persistence + migrations.
- `cpt-insightspec-actor-api-gateway` — sole external caller in Phase 1.

#### Insight.Identity.Domain

- [ ] `p1` - **ID**: `cpt-insightspec-component-identity-domain`

##### Why this component exists

To carry the lookup orchestration and observation-collapse logic in
a layer that has zero compile-time coupling to ASP.NET Core,
MySqlConnector, or DbUp. This is what makes unit tests of
`PersonAssembler` and `DisplayNameSplitter` fast (~20 tests run in
~20 ms) and what makes the algorithm legible in isolation from the
SQL strings.

##### Responsibility scope

- `PersonLookupService.GetByEmailAsync(tenant, email)` —
  trims the email, resolves `person_id` (case-insensitive via
  the column collation per ADR-0011), fetches latest-per-source
  observations, hands them to the assembler.
- `PersonAssembler.Assemble(observations)` — collapses per-`value_type`
  observations across sources by latest `created_at`, falls back to
  `DisplayNameSplitter` when `first_name`/`last_name` are absent.
- `DisplayNameSplitter.Split(displayName)` — handles `"Last, First"`
  and `"First Last"` formats; single-token names yield
  `(token, "")`.
- `ValueTypes` — canonical attribute constants used by the assembler.
- Ports: `IPersonsReader`, `ITenantContext`.

##### Responsibility boundaries

- Does **not** open MariaDB connections — that's
  `MariaDbConnectionFactory` in Infrastructure.
- Does **not** know which `value_type` routes to which physical
  column — that's the seed pipeline's contract (ADR-0007) and the
  repository's SQL.
- Does **not** map results to JSON — that's Api's serialiser.

##### Related components (by ID)

- `cpt-insightspec-component-identity-api` — consumes the lookup
  service.
- `cpt-insightspec-component-identity-infra` — implements
  `IPersonsReader`.

#### Insight.Identity.Infrastructure

- [ ] `p1` - **ID**: `cpt-insightspec-component-identity-infra`

##### Why this component exists

To isolate every MariaDB-specific detail (connection-string parsing,
`BINARY(16)` parameter binding, `ROW_NUMBER()` CTE, DbUp migration
runner) in one project so the Domain code stays portable and so a
future read replica or backup target can be swapped in without
touching the lookup algorithm.

##### Responsibility scope

- `MariaDbConnectionFactory` — parses `mysql://user:pass@host:port/db`
  with an explicit regex (deliberately avoiding `System.Uri`'s
  generic-scheme rewrites), exposes the resolved `ConnectionString`
  and the sanitised `Target` (`host:port/db`, no creds) for log
  context.
- `PersonsRepository` — implements `IPersonsReader`; binds Guids as
  `BINARY(16)` bytes; materialises `PersonObservation` rows.
- `Sql` — centralised constants for the two queries
  (`ResolvePersonIdByEmail`, `LatestObservationsByPersonId`); the CTE
  is one of the documented SQL artefacts (see §3.7).
- `MigrationRunner` — DbUp 6.x adapter, embeds SQL via
  `WithScriptsEmbeddedInAssembly`, bridges DbUp's `IUpgradeLog` to
  `Microsoft.Extensions.Logging.ILogger`.

##### Responsibility boundaries

- Does **not** decide tenant routing or display-name fallback —
  that's Domain.
- Does **not** emit HTTP responses — that's Api.
- Does **not** orchestrate the seed pipeline — that's
  `src/backend/services/identity/seed/`.

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
| [`cpt-insightspec-interface-identity-person-lookup`](PRD.md#get-v1personsemail--person-lookup) | `PersonsEndpoints.GetByEmail` in `Insight.Identity.Api/Endpoints/PersonsEndpoints.cs`. Snake-case JSON via configured `JsonSerializerOptions`. | Phase 2 will add a POST counterpart; the GET stays. |
| [`cpt-insightspec-interface-identity-health`](PRD.md#get-health--database-readiness) | `PersonsEndpoints.Health` — opens a connection, runs `SELECT 1`. | 200 / 503. |
| [`cpt-insightspec-interface-identity-healthz`](PRD.md#get-healthz--process-liveness) | Inline `MapGet("/healthz", ...)` returning `"ok"`. | Never touches DB. |

External contracts:

- [`cpt-insightspec-contract-identity-env-config`](PRD.md#identity_-env-var-contract) —
  honoured by `Microsoft.Extensions.Configuration.EnvironmentVariables`
  with prefix `IDENTITY__` and `__` section delimiter; bound to
  strongly-typed `AppOptions` / `MariaDbOptions` records.
- [`cpt-insightspec-contract-identity-config-secret`](PRD.md#insight-identity-config-secret) —
  consumed via `envFrom: secretRef: insight-identity-config` in the
  Deployment template (see `src/backend/services/identity/helm/`).

### 3.4 Internal Dependencies

| Dependency Module | Interface Used | Purpose |
|-------------------|----------------|---------|
| `Insight.Identity.Domain` | `IPersonsReader`, `ITenantContext`, `PersonLookupService`, `PersonAssembler`, `ValueTypes` | Lookup orchestration + observation collapse. |
| `Insight.Identity.Infrastructure` | `PersonsRepository`, `MariaDbConnectionFactory`, `MigrationRunner` | MariaDB persistence + DbUp migrations. |
| `charts/insight/templates/secrets.yaml` (umbrella) | Emits `insight-identity-config` with `IDENTITY__mariadb__url` etc. | Runtime config supply. |
| `charts/insight/templates/mariadb-initdb-scripts.yaml` (umbrella) | Provisions empty `identity` database + grants on first MariaDB pod boot. | Empty DB substrate for DbUp to migrate. |

### 3.5 External Dependencies

| Dependency | Version | Why | Failure mode |
|------------|---------|-----|--------------|
| MySqlConnector (NuGet) | 2.4.0 | MariaDB-flavoured wire protocol; `MySqlDbType.Binary` for `BINARY(16)` Guid binding. | Pool exhaustion → 503 on `/health`; pod restart on transient connectivity loss. |
| dbup-core + dbup-mysql (NuGet) | 6.0.4 | Schema migration applied at startup; tracks `SchemaVersions`. | Failed migration → exception thrown, pod crashes before listener opens. |
| Serilog + Serilog.Formatting.Compact + Serilog.AspNetCore (NuGet) | 9.x | Structured JSON logs with `CompactJsonFormatter`, request-logging middleware, PII redaction. | Logger init failure → pod crashes; no fallback. |
| Microsoft.AspNetCore.Mvc.Testing | 9.0.0 (test only) | `WebApplicationFactory` for integration tests. | n/a — test-only. |
| Testcontainers.MariaDb | 4.11.0 (test only) | Spins up a real MariaDB per integration test collection. | Test failure when Docker unavailable; not a runtime concern. |

### 3.6 Interactions & Sequences

#### Person lookup happy path

- [ ] `p1` - **ID**: `cpt-insightspec-seq-identity-lookup-happy`

```
api-gateway  →  identity-api  →  CompositeTenantContext  →  PersonLookupService
                                                              │
                                                              ▼
                              IPersonsReader.ResolvePersonIdByEmailAsync
                                                              │
                                              (covered idx_value_id)
                                                              ▼
                              IPersonsReader.GetLatestObservationsAsync
                                                              │
                                          (ROW_NUMBER OVER PARTITION)
                                                              ▼
                                              PersonAssembler.Assemble
                                                              │
                                                              ▼
                                                  PersonResponse (JSON)
                                                              │
                                                              ▼
                                                       api-gateway merges
```

1. api-gateway calls `GET /v1/persons/alice@example.com` with
   `X-Insight-Tenant-Id: 01933a40-...` (UUID).
2. `CompositeTenantContext.Resolve` reads the header → `Guid`.
3. `PersonLookupService.GetByEmailAsync` trims the email (case
   handled at the storage layer per ADR-0011).
4. `PersonsRepository.ResolvePersonIdByEmailAsync` issues
   `SELECT person_id FROM persons WHERE insight_tenant_id=@t AND
   value_type='email' AND value_id=@email ORDER BY created_at DESC,
   id DESC LIMIT 1` on the `idx_value_id` covered index.
5. `PersonsRepository.GetLatestObservationsAsync` runs the
   `ROW_NUMBER()` CTE, returning one row per (source, value_type).
6. `PersonAssembler.Assemble` collapses across sources by latest
   `created_at`, runs `DisplayNameSplitter` if first/last absent.
7. `PersonsEndpoints` serialises to snake-case JSON; returns 200.

#### Tenant unresolved

- [ ] `p1` - **ID**: `cpt-insightspec-seq-identity-tenant-unresolved`

```
caller  →  identity-api  →  CompositeTenantContext.Resolve()
                                       │
                                  (all return null)
                                       │
                                       ▼
                            Results.Problem(...)
                                       │
                                       ▼
                       400 + RFC 7807 problem-details
```

The composite walks header → JWT stub → config default; if all return
null, the endpoint returns
`urn:insight:error:tenant_unresolved` with status 400.

#### Startup with migration

- [ ] `p1` - **ID**: `cpt-insightspec-seq-identity-startup`

```
kubelet  →  pod start  →  Program.cs Configuration bind
                                    │
                                    ▼
                          MariaDbConnectionFactory init
                                    │
                                    ▼
                          MigrationRunner.Run
                            │      EnsureDatabase.For.MySqlDatabase
                            │      DeployChanges.To.MySqlDatabase
                            │      WithScriptsEmbeddedInAssembly("*.Migrations.*.sql")
                            ▼
                          PerformUpgrade()
                            │      (failure → throw, pod restart)
                            ▼
                          app.RunAsync()
                                    │
                                    ▼
                          /health, /healthz, /v1/persons/{email}
```

DbUp's `SchemaVersions` table guarantees each script applies once
across pod restarts; idempotency is at the script level (every DDL
uses `CREATE TABLE IF NOT EXISTS`).

### 3.7 Database schemas & tables

The service is a **reader** of `persons` and the migrator of the
`identity` MariaDB database.

#### Table: `persons` (MariaDB)

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-identity-persons`

Defined in `Insight.Identity.Infrastructure/Migrations/001_persons.sql`
(applied at service startup via DbUp). Canonical column reference:
[docs/domain/identity-resolution/specs/DESIGN.md §"Table: persons"](../../../../../domain/identity-resolution/specs/DESIGN.md#table-persons-mariadb).

The service reads it via two queries (both in
`Insight.Identity.Infrastructure/MariaDb/Sql.cs`):

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

Defined in `Insight.Identity.Infrastructure/Migrations/002_account_person_map.sql`.
The service migrates the table at startup but does **not** read it in
Phase 1 — the seed pipeline rebuilds it as an SCD2 cache from
`persons` (see
[domain DESIGN §"Table: account_person_map"](../../../../../domain/identity-resolution/specs/DESIGN.md#table-account_person_map-mariadb)).
Future Phase 2 lookups will use it for "as-of" account → person
binding queries.

#### Table: `org_chart` (MariaDB)

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-identity-org-chart`

Defined in `Insight.Identity.Infrastructure/Migrations/003_org_chart.sql`
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
- `IPersonsReader.GetCurrentParentsAsync(tenant, child)` —
  `WHERE child_person_id=@c AND valid_to IS NULL` against
  `idx_current_parent`.
- `IPersonsReader.GetCurrentChildrenAsync(tenant, parent)` —
  `WHERE parent_person_id=@p AND valid_to IS NULL` against
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
row set ordered by depth; the C# service layer (`SubchartService`)
assembles the tree by indexing on `parent_person_id`. Visibility is
applied via `VisibilityService.CanSeeAsync` on the root only — the
visibility CTE is closed under `org_chart` descent, so once the
viewer can see the root every descendant is already in their
visible set.

#### Table: `SchemaVersions` (MariaDB, DbUp-managed)

- [ ] `p1` - **ID**: `cpt-insightspec-dbtable-identity-schema-versions`

DbUp's tracker table. Created automatically on first
`PerformUpgrade()` if absent; the service does not interact with it
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
   `DBNull` for the parameter rather than passing C#'s
   `DateTime.UtcNow`; both timestamps in a subsequent
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
   (never NULL), short-circuits at the planner, and reads as
   `Convert.ToBoolean(scalar)` in C#.

5. **BINARY(16) UUID round-trip** uses big-endian RFC 4122 wire
   order: `guid.ToByteArray(bigEndian: true)` when binding, and
   `new Guid(bytes, bigEndian: true)` when reading. Established
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

| Env var | Default | Notes |
|---------|---------|-------|
| `IDENTITY__mariadb__url` | _none_ (required) | `mysql://user:pass@host:port/db`; percent-encoding allowed for users / passwords. Mutually exclusive with `connection_string`. |
| `IDENTITY__mariadb__connection_string` | _none_ | Raw MySqlConnector KV form for callers needing options the URL shape cannot express. |
| `IDENTITY__mariadb__min_pool_size` | 0 | Lazily opens connections. |
| `IDENTITY__mariadb__max_pool_size` | 16 | Smaller than the analytics service per design review. |
| `IDENTITY__identity__bind_addr` | `0.0.0.0:8082` | Listener address. |
| `IDENTITY__identity__tenant_default_id` | _empty_ | Optional; opt-in for single-tenant clusters. |
| `IDENTITY__identity__expand_subordinates` | `false` | Phase 2 toggle (recursive supervisor walk). |

### 4.2 Logging shape

Every log line is structured JSON via `CompactJsonFormatter` with:

- `@t` — RFC 3339 timestamp.
- `@l` — level.
- `@mt` — message template (e.g. `"HTTP {RequestMethod} {RequestPath} responded {StatusCode} in {Elapsed:0.0000} ms"`).
- `@tr` / `@sp` — W3C trace and span IDs (when present).
- `service` — `identity` (Serilog enricher).
- `RequestPath` — route template, never the raw email path.
- For unhandled exceptions: `@x` carries the full stack;
  `db_target` is set on the diagnostic context only when the
  exception is a `MySqlException` / `DbException`.

## 5. Traceability

| PRD ID | DESIGN reference |
|--------|------------------|
| `cpt-insightspec-fr-identity-lookup-resolve-by-email` | §1.2 Functional Drivers; §3.7 SQL `ResolvePersonIdByEmail`. |
| `cpt-insightspec-fr-identity-lookup-hydrate` | §1.2 Functional Drivers; §3.7 SQL `LatestObservationsByPersonId`. |
| `cpt-insightspec-fr-identity-lookup-404` | §1.2 Functional Drivers; §3.3 API Contracts. |
| `cpt-insightspec-fr-identity-lookup-400-tenant` | §1.2 Functional Drivers; §3.6 Sequence "Tenant unresolved". |
| `cpt-insightspec-fr-identity-lookup-parent` | §1.2 Functional Drivers; `PersonLookupService.ResolveParentAsync` + `PersonAssembler.Assemble`. |
| `cpt-insightspec-fr-identity-lookup-subordinates` | §1.2 Functional Drivers; `PersonLookupService.HydrateAsync` recursion + `IPersonsReader.GetCurrentChildrenAsync`. |
| `cpt-insightspec-fr-identity-routing-name-split` | §1.2 Functional Drivers; §3.2 Domain `DisplayNameSplitter`. |
| `cpt-insightspec-fr-identity-migrations-startup` | §1.2 Functional Drivers; §3.6 Sequence "Startup with migration". |
| `cpt-insightspec-fr-identity-schema-relax-uniqueness` | §1.2 Functional Drivers; ADR-0011 §Decision Outcome (new UNIQUE on `created_at`). |
| `cpt-insightspec-fr-identity-schema-case-insensitive-value-id` | §1.2 Functional Drivers; ADR-0011 §Decision Outcome (collation switch to `utf8mb4_unicode_ci`). |
| `cpt-insightspec-fr-identity-org-chart-table` | §1.2 Functional Drivers; §3.7 Table `org_chart`; ADR-0010. |
| `cpt-insightspec-fr-identity-org-chart-rebuild` | §1.2 Functional Drivers; rebuild step in seeder (`seed-persons-from-identity-input.py` step 9). |
| `cpt-insightspec-fr-identity-org-chart-read` | §1.2 Functional Drivers; §3.7 read paths note; `SqlOrgChart` + `PersonsRepository.ReadEdgesAsync`. |
| `cpt-insightspec-fr-identity-profile-org-tree` | §1.2 Functional Drivers; `ProfileLookupService.ResolveAsync` → `PersonLookupService.HydrateForProfileAsync` → `ProfileAssembler.Assemble`. |
| `cpt-insightspec-nfr-identity-latency` | §1.2 NFR Allocation; §3.7 covered index. |
| `cpt-insightspec-nfr-identity-memory` | §1.2 NFR Allocation; §2.1 Principle "Observation log, not relational tree". |
| `cpt-insightspec-nfr-identity-logging-pii` | §1.2 NFR Allocation; §4.2 Logging shape. |
| `cpt-insightspec-nfr-identity-uuid-roundtrip` | §1.2 NFR Allocation; §2.2 Constraint "BINARY(16) for every UUID". |
| `cpt-insightspec-interface-identity-person-lookup` | §3.3 API Contracts. |
| `cpt-insightspec-interface-identity-health` | §3.3 API Contracts. |
| `cpt-insightspec-interface-identity-healthz` | §3.3 API Contracts. |
| `cpt-insightspec-contract-identity-env-config` | §3.3 API Contracts; §4.1 Configuration surface. |
| `cpt-insightspec-contract-identity-config-secret` | §3.3 API Contracts; §3.4 Internal Dependencies. |
