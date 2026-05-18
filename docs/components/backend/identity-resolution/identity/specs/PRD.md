# PRD — Identity

<!-- toc -->

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
  - [5.1 Lookup contract](#51-lookup-contract)
  - [5.2 Profile lookup (POST /v1/profiles, Phase 2 — #347)](#52-profile-lookup-post-v1profiles-phase-2--347)
  - [5.3 Routing and normalisation](#53-routing-and-normalisation)
  - [5.4 Schema lifecycle](#54-schema-lifecycle)
  - [5.5 Parent/child edge cache](#55-parentchild-edge-cache)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 NFR Inclusions](#61-nfr-inclusions)
  - [6.2 NFR Exclusions](#62-nfr-exclusions)
- [7. Public Library Interfaces](#7-public-library-interfaces)
  - [7.1 Public API Surface](#71-public-api-surface)
  - [7.2 External Integration Contracts](#72-external-integration-contracts)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

`insight-identity` is a .NET 9 / ASP.NET Core minimal-API service that
serves person lookups over the multi-source observation log stored in
the MariaDB `persons` table. It owns its database (per ADR-0006),
applies its own DbUp migrations at startup, and exposes a small
read-only HTTP surface to api-gateway and internal workflows
(`GET /v1/persons/{email}`, `/health`, `/healthz`).

The service is the first synchronous consumer of the append-only
observation log seeded from `identity.identity_inputs`. It enriches
analytics responses with display names, supervisor links, and other
person attributes — without callers having to know which connector
provided each value.

### 1.2 Background / Problem Statement

Identity-bearing data lands in Insight from multiple connectors —
BambooHR, Cursor, Claude Admin, Jira, Slack, MS Entra, and others. PR
#214 introduced an append-only `persons` observation log that unifies
every connector behind a single schema (one row per
`(insight_tenant_id, person_id, insight_source_type, insight_source_id,
value_type, value_hash)`). Until this service shipped, the only
synchronous consumer was a Rust stub that loaded BambooHR `employees`
into an in-memory `HashMap` at startup — single-source, restart-coupled
to the bronze snapshot, and unable to surface non-BambooHR observations.

The platform needs a synchronous lookup path that (a) sees every source
the seed pipeline writes, (b) returns live data without a pod restart,
(c) is tenant-safe by construction, and (d) follows the cyberfabric
ASP.NET Core / Serilog / RFC 7807 conventions established for other
.NET services in the platform.

### 1.3 Goals (Business Outcomes)

- **Multi-source coverage.** Lookup answers correctly for any source
  whose connector emits identity observations — not only BambooHR.
- **Live data.** Updates that land in `persons` are visible without a
  pod restart; no in-memory full-table cache to invalidate.
- **Tenant safety.** Every query is scoped by `insight_tenant_id`; no
  cross-tenant data leak is possible by construction.
- **Operational predictability.** First-install behaviour is "every
  lookup returns 404 until the seed runs" — never a crash loop.

### 1.4 Glossary

| Term | Definition |
|------|------------|
| `persons` | The MariaDB append-only observation log; one row per (tenant, person_id, source_type, source_id, value_type, created_at) per ADR-0011. The earlier UNIQUE on `value_hash` wrongly collapsed state transitions and was dropped. Defined in `identity` DB. |
| `account_person_map` | SCD2 cache derived from `persons` rows where `value_type='id'`; maps source-account → `person_id` over time. |
| Observation | One row in `persons` — a single (`value_type`, `value`) datapoint emitted by one source for one person at one instant. Never updated; superseded by a newer observation with the same partition key. |
| `value_type` | Free-form `VARCHAR(50)` attribute name. Canonical set: `id`, `email`, `username`, `display_name`, `first_name`, `last_name`, `department`, `division`, `job_title`, `status`, `employee_id`, `parent_email`, `parent_id`, `parent_person_id`. |
| `value_id` / `value_full_text` / `value` | Routing columns selected per `value_type` per ADR-0007. `id`/`email`/`username` → `value_id` (utf8mb4_unicode_ci — case-insensitive per ADR-0011); `display_name` → `value_full_text` (utf8mb4_unicode_ci); everything else → `value`. |
| `insight_tenant_id` | `BINARY(16)` tenant UUID; part of every query and every index. |
| Latest-per-source | The projection `ROW_NUMBER() OVER (PARTITION BY source_type, source_id, value_type ORDER BY created_at DESC)` — picks the most recent observation per attribute per source. |
| Assembler | `PersonAssembler` — collapses latest-per-source rows into a single `PersonResponse` by picking the latest value across sources per `value_type`. |
| DbUp | The .NET migration library; tracks applied SQL scripts in a `SchemaVersions` table inside the service's own database. |
| Seed | The one-shot Bash + Python pipeline at `src/backend/services/identity/seed/` that materialises `persons` rows from ClickHouse `identity.identity_inputs`. Not a schema migration. |

## 2. Actors

### 2.1 Human Actors

#### Platform SRE

**ID**: `cpt-insightspec-actor-platform-sre`

**Role**: Operates the Insight install on a customer cluster. Runs
seed pipelines, reads `/health` and `/healthz` to determine pod
readiness, and triages 5xx responses from the service.

**Needs**: A deterministic health/readiness contract; structured logs
that name the failure mode without leaking PII; a clear error response
when the seed has not yet been run.

#### Connector Developer

**ID**: `cpt-insightspec-actor-identity-connector-dev`

**Role**: Adds new connectors that emit identity observations and
extends the `value_type` taxonomy. Validates that new attributes
surface correctly on the lookup response.

**Needs**: A stable contract for which `value_type`s are projected;
documented routing rules (ADR-0007); a way to extend the projection
without breaking existing callers.

### 2.2 System Actors

#### api-gateway

**ID**: `cpt-insightspec-actor-api-gateway`

**Role**: External-facing reverse proxy. Calls
`GET /v1/persons/{email}` to enrich analytics responses with
display-name, supervisor, and org-unit fields. Sends
`X-Insight-Tenant-Id` derived from the resolved JWT principal.

#### dbt-runner / Argo Workflows

**ID**: `cpt-insightspec-actor-identity-argo`

**Role**: Internal compute callers that may need person metadata when
materialising Gold tables or running ad-hoc reconciliations. Carry
the tenant context via the same header.

#### MariaDB

**ID**: `cpt-insightspec-actor-mariadb`

**Role**: Stores the `persons` and `account_person_map` tables that
the service reads and that DbUp migrates on startup. Connection
target named by `IDENTITY__mariadb__url`.

#### Seed pipeline

**ID**: `cpt-insightspec-actor-seed-pipeline`

**Role**: Writes observation rows into `persons` from ClickHouse
`identity.identity_inputs`. Runs out-of-band (operator-triggered);
the service does not orchestrate it. The reader trusts that any
visible row is well-formed per the routing rules in ADR-0007.

## 3. Operational Concept & Environment

### 3.1 Module-Specific Environment Constraints

- **.NET 9 runtime.** Service binary is published as
  `linux/amd64` self-contained; Kubernetes pod runs as UID 1000
  non-root.
- **MariaDB reachability at startup.** DbUp connects, runs
  `EnsureDatabase`, applies migrations, then opens the HTTP listener.
  If MariaDB is unreachable, the pod crashes early — kubelet retries.
  There is no "start without DB and reconnect later" mode.
- **No in-memory cache.** Every lookup hits MariaDB. Memory budget
  (NFR-2) reflects the absence of cache, not its presence.
- **Tenant header mandatory in prod.** With `tenant_default_id`
  unset, every request must carry `X-Insight-Tenant-Id`. Dev / local
  clusters pin a default tenant in values; production overlays leave
  it empty (the validator in api-gateway derives it from the JWT
  principal before forwarding).

## 4. Scope

### 4.1 In Scope

- `GET /v1/persons/{email}` (Phase 1) returning a single
  `PersonResponse` with parent attributes (`parent_email`,
  `parent_id`, `parent_person_id`) but no recursive subordinate
  expansion. Preserved unchanged by Phase 2.
- `POST /v1/profiles` (Phase 2, cyberfabric/cyber-insight#347)
  — single-profile lookup by either email (across all sources) or
  source-native id (within one source instance), returning a
  `ProfileResponse` with the full `ids[]` list of current
  `value_type='id'` bindings. Single-result invariant enforced;
  multiple matches surface as `422 urn:insight:error:ambiguous_profile`.
- `GET /health` — DB ping (200 if reachable, 503 otherwise).
- `GET /healthz` — process liveness (200 `text/plain "ok"`).
- Tenant resolution by `X-Insight-Tenant-Id` header with optional
  fallback to `IDENTITY__identity__tenant_default_id` config.
  Same `CompositeTenantContext` used by both endpoints.
- Lowercase-email lookup against `value_type = 'email'`.
- Display-name split fallback when explicit `first_name` /
  `last_name` observations are absent.
- DbUp-applied schema (`001_persons.sql`, `002_account_person_map.sql`)
  per ADR-0006.

### 4.2 Out of Scope

- Recursive subordinate expansion via `parent_person_id` —
  cyberfabric/cyber-insight#348 (GET subchart) lands separately.
- Real JWT-claim validation (Phase 2.5 — `JwtTenantContext` is wired
  in DI as a stub, returns `null`; api-gateway BFF forwarding lands
  this).
- Batch (multi-lookup) profile resolution — Phase 2 surfaces a single
  lookup per request; multi-lookup body shape is a possible Phase 3
  extension.
- Temporal "as-of" queries by date range — Phase 3.
- Write path (`POST /v1/resolve` golden-record bootstrap) —
  cyberfabric/cyber-insight#349.
- Writing observations into `persons` (owned by the seed pipeline
  and a future reconciliation service).
- Merge / split workflows on person identities.
- OIDC subject mapping, org_units, memberships, user_identities,
  user_roles tables — tracked separately under cyberfabric/cyber-insight#80.

## 5. Functional Requirements

> **Testing strategy**: All functional requirements verified via
> automated tests — unit tests cover domain logic (`PersonAssembler`,
> `DisplayNameSplitter`, `MariaDbConnectionFactory`); integration
> tests cover SQL + endpoint behaviour against a Testcontainers
> MariaDB.

### 5.1 Lookup contract

#### Resolve email to person_id

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-lookup-resolve-by-email`

The system **MUST** resolve an email to a single `person_id` using
the latest observation per
`(insight_source_type, insight_source_id, value_type, value_id)`
partition where `value_type = 'email'` and `insight_tenant_id`
matches. The comparison is case-insensitive at the storage layer
(ADR-0011); the caller need not lowercase the input.

**Rationale**: Email is the lookup key used by every current caller;
"latest per source" matches the seed pipeline's semantics and avoids
returning stale post-merge identities.

**Actors**: `cpt-insightspec-actor-api-gateway`,
`cpt-insightspec-actor-argo-workflows`

#### Hydrate person attributes

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-lookup-hydrate`

The system **MUST** hydrate every other response field with the latest
observation per `(insight_source_type, insight_source_id, value_type)`
partition for the resolved `person_id`. The assembler **MUST** then
pick the per-`value_type` winner across sources by latest `created_at`.

**Rationale**: A single source can be authoritative for some fields
and silent on others; the assembler must compose the response from
multiple sources without preferring any one of them by default.

**Actors**: `cpt-insightspec-actor-api-gateway`

#### Not-found returns RFC 7807

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-lookup-404`

The system **MUST** return `404 Not Found` with an RFC 7807
problem-details body when no current observation matches the supplied
email + tenant.

**Rationale**: Empty-result is a normal first-install state, not an
error; callers must distinguish it from server failures.

**Actors**: `cpt-insightspec-actor-api-gateway`

#### Missing tenant returns RFC 7807

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-lookup-400-tenant`

The system **MUST** return `400 Bad Request` with an RFC 7807
problem-details body of type
`urn:insight:error:tenant_unresolved` when the request carries no
`X-Insight-Tenant-Id` header and no `tenant_default_id` is configured.

**Rationale**: Silently defaulting a tenant in a multi-tenant
deployment is a data-leak risk. The composite resolver lets the header
win; the default is opt-in for single-tenant clusters.

**Actors**: `cpt-insightspec-actor-platform-sre`

#### Surface parent attributes when present

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-lookup-parent`

The system **MUST** surface `parent_email`, `parent_id`, and
`parent_person_id` on the response when those `value_type`s exist for
the resolved person.

**Rationale**: Org-tree consumers need the supervisor edge today even
though recursive expansion is Phase 2; surfacing the raw observations
unblocks them without coupling to the reconciliation service.

**Actors**: `cpt-insightspec-actor-api-gateway`

### 5.2 Profile lookup (POST /v1/profiles, Phase 2 — #347)

#### Resolve profile by email or source-native id

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-profile-resolve`

The system **MUST** expose `POST /v1/profiles` accepting a JSON body
of shape `{ value_type, value, insight_source_type?, insight_source_id? }`
and returning a single `ProfileResponse` when exactly one current
observation matches.

The contract has two valid request shapes:
- `value_type='email'` — `value` is the email to look up across ALL
  source instances for the tenant. `insight_source_type` and
  `insight_source_id` **MUST** be absent.
- `value_type='id'` — `value` is the source-native account id from a
  `persons.value_type='id'` observation. Both `insight_source_type`
  and `insight_source_id` **MUST** be supplied.

The handler resolves over the canonical latest-per-source-instance
partition `(insight_tenant_id, person_id, insight_source_type,
insight_source_id, value_type)`; observations superseded by a newer
row on the same partition do not contribute.

**Rationale**: Phase 1 GET endpoint is limited to email lookup; the
analytics front-end and internal workflows need to resolve by other
identifier types (source-native id especially for the
person-by-source workflows in cyberfabric/cyber-insight#344). POST
with a structured body keeps the contract extensible for Phase 3
date-range filtering without further URL gymnastics.

**Actors**: `cpt-insightspec-actor-api-gateway`,
`cpt-insightspec-actor-identity-argo`

#### Surface single-result invariant via 422

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-profile-ambiguous-422`

When `POST /v1/profiles` matches more than one distinct `person_id`
on the same lookup, the system **MUST** return `422 Unprocessable
Entity` with an RFC 7807 body of type
`urn:insight:error:ambiguous_profile`. The body **MUST** echo the
offending lookup verbatim and include the list of matched
`person_ids` so the caller can investigate the data invariant
violation without re-querying.

**Rationale**: The data invariant is "exactly one current person
per source-instance id, and exactly one current person per email
across all sources for a tenant". A violation indicates a corrupted
`persons` table state; silently picking one record would mask the
problem and risk wrong-person responses. 422 (RFC 9110 §15.5.21
"semantically correct request, server cannot process due to
data state") matches the cyberfabric platform convention used by
analytics-api for similar invariant breaches.

**Actors**: `cpt-insightspec-actor-platform-sre`

#### Project full alias list on response

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-profile-ids-list`

The `ProfileResponse` **MUST** include an `ids[]` array enumerating
every current `value_type='id'` binding for the resolved person, one
entry per `(insight_source_type, insight_source_id)` instance. Each
entry has the shape
`{ insight_source_type, insight_source_id, value }` and corresponds
to the latest observation on its partition.

**Rationale**: Consumers downstream (analytics enrichment, future
front-end org-tree) need the full alias picture without making N
follow-up lookups per source.

**Actors**: `cpt-insightspec-actor-api-gateway`

#### Validate request body via FluentValidation

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-profile-validation`

The system **MUST** reject malformed `POST /v1/profiles` bodies with
`400 Bad Request` + RFC 7807 body before reaching the persistence
layer. The error type **MUST** be one of:
`urn:insight:error:invalid_value_type`,
`urn:insight:error:invalid_value`,
`urn:insight:error:missing_source_for_id`,
`urn:insight:error:source_not_allowed_for_email`.

Cross-field rules are expressed via FluentValidation's `When(...)`
predicates; the validator is registered in DI via
`AddValidatorsFromAssemblyContaining<…>`.

**Rationale**: Cross-field validation (`value_type='id'` requires
both source fields; `value_type='email'` forbids them) is not
ergonomically expressible with Data Annotations; FluentValidation
keeps the rules in one class that is unit-testable independently of
the endpoint.

**Actors**: `cpt-insightspec-actor-api-gateway`,
`cpt-insightspec-actor-platform-sre`

### 5.3 Routing and normalisation

#### Display-name split fallback

- [x] `p2` - **ID**: `cpt-insightspec-fr-identity-routing-name-split`

The system **MUST** fall back to splitting `display_name` into
`first_name` / `last_name` when neither explicit observation is
present, using the rules in ADR-0006 (`"Last, First"` vs
`"First Last"`).

**Rationale**: BambooHR's older snapshot lacked dedicated first/last
fields; the split keeps the response shape complete without forcing
a connector backfill.

**Actors**: `cpt-insightspec-actor-api-gateway`

### 5.4 Schema lifecycle

#### Service-owned migrations at startup

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-migrations-startup`

The service **MUST** apply its own DbUp migrations (plain SQL files
under `Insight.Identity.Infrastructure/Migrations/`) against the
configured MariaDB before opening the HTTP listener. Migration history
**MUST** be tracked in a `SchemaVersions` table inside the service's
own database.

**Rationale**: Per ADR-0006 each service owns its schema; serial
startup ordering prevents requests from ever hitting an unmigrated
table.

**Actors**: `cpt-insightspec-actor-mariadb`, `cpt-insightspec-actor-platform-sre`

#### Schema allows recording state transitions

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-schema-relax-uniqueness`

The `persons` table **MUST** record every observation event, including
a value reverting to a prior value at a different point in time
(`Active → Inactive → Active`). The UNIQUE constraint **MUST NOT**
include `value_hash` (which collapsed return-to-prior-value
transitions); it **MUST** use `created_at` as the disambiguator so
re-runs of the seeder against the same source snapshot remain
idempotent while genuine state transitions are preserved.

**Rationale**: Per ADR-0011, the original UNIQUE on `value_hash` was
a design mistake — it conflated "same observation re-emitted on a
re-run" (which should dedupe) with "same value observed again at a
later time" (which should not). The fix is structural: drop the
value-hash-based UNIQUE and re-key on `created_at`.

**Actors**: `cpt-insightspec-actor-mariadb`,
`cpt-insightspec-actor-seed-pipeline`

#### Value comparisons are case-insensitive

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-schema-case-insensitive-value-id`

All `value_id`-routed value_types (`id`, `email`, `username`,
`employee_id`, `parent_email`, `parent_id`, `parent_person_id`)
**MUST** compare case-insensitively. The column collation **MUST**
be `utf8mb4_unicode_ci` so the comparison applies uniformly at the
storage layer; SQL callers **MUST NOT** be required to wrap reads
in `LOWER()` to get the right answer.

**Rationale**: Per ADR-0011, the original `utf8mb4_bin` collation
on `value_id` made every comparison strictly case-sensitive — a
production lookup for `Alice.Smith@company.com` against a
stored `alice.smith@company.com` returned 404. Switching the
column to `utf8mb4_unicode_ci` aligns the storage with how the
platform conventionally compares emails and UUID strings, and
removes a fragile per-caller contract.

**Actors**: `cpt-insightspec-actor-api-gateway`,
`cpt-insightspec-actor-mariadb`

### 5.5 Parent/child edge cache

Phase 1 of cyberfabric/cyber-insight#348 — storage layer for
organisational tree relationships. No API surface change in Phase 1;
the cache is read by Phase 2 endpoint enrichment and Phase 3 subchart
walks.

The cache depends on **`value_type='status'`** observations to close
edges on employee deactivation (see `cpt-insightspec-fr-identity-org-chart-rebuild`
below). The canonical value_type set the rebuild reads is therefore
`parent_email`, `parent_person_id`, `email`, and `status` — these
must continue to be enumerable as expected `value_type` values in
new connector dbt models.

**Multi-parent extensibility.** Phase 1 enforces single-parent per
`(tenant, source_type, source_id, child)`. The schema can be promoted
to multi-parent if a future source emits multiple supervisors per
employee (matrix orgs) by adding `parent_person_id` to the primary
key. The read API contract is already a list, so no consumer-side
change is required. No source today produces multi-parent data; this
is captured for future-readers, not implemented in Phase 1.

#### Materialised parent/child edge cache

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-org-chart-table`

The service **MUST** own a `org_chart` table that stores
direct parent->child edges per
`(insight_tenant_id, insight_source_type, insight_source_id)` and
keeps SCD2 history via `valid_from`/`valid_to`. The Phase-1 invariant
is at most one CURRENT edge per
`(tenant, source_type, source_id, child_person_id)`; multi-parent
support (matrix orgs) is deferred to a Phase-1.5 schema change that
adds `parent_person_id` to the primary key.

**Rationale**: Per-source edges are first-class — Alice's manager in
BambooHR is not the same edge as her channel admin in Slack — and a
dedicated cache lets the Phase-3 recursive subchart endpoint be an
index walk rather than a partition-arithmetic-inside-recursion query.

**Actors**: `cpt-insightspec-actor-mariadb`,
`cpt-insightspec-actor-seed-pipeline`

#### Rebuild edges from persons deterministically

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-org-chart-rebuild`

The seeder (`seed-persons-from-identity-input.py` step 9) **MUST**
rebuild `org_chart` from `persons` using the same two-table-
swap pattern as `account_person_map`: build into
`org_chart_next`, atomically `RENAME TABLE` into place, drop
the old artefact. The rebuild **MUST** UNION two sources of edges:

1. `value_type='parent_person_id'` observations (resolved Insight
   UUIDs that a future reconciliation service will write; currently
   zero rows but the path stays live).
2. `value_type='parent_email'` observations resolved by JOIN to the
   latest `value_type='email'` observation per (tenant, email)
   partition. The JOIN **MUST** lowercase + trim the
   `parent_email` side and **MUST** pick at most one `person_id`
   per (tenant, email) via `ROW_NUMBER() OVER (...) WHERE rn=1`
   ordered by `created_at DESC` so pending-iresolution
   accumulation cannot break the UNIQUE key.

Source 1 **MUST** take precedence over Source 2 when both have a
row for the same `(tenant, person, source_type, source_id)`
partition (NOT EXISTS guard). Malformed `value_id`s in Source 1
(not a canonical 36-char UUID) and self-loops in both sources
**MUST** be skipped pre-insert. Parent_emails that do not match any
current email-bearer in the tenant **MUST** be skipped and counted
in the seeder log; the seeder **MUST NOT** synthesise stub persons
to carry an unresolved `parent_email`.

Source 2 **MUST** intersect each `parent_email` observation period
with the child's active intervals derived from `value_type='status'`
observations:

- An active interval starts at any `status` observation whose value
  is not Inactive/Terminated and the previous observation was either
  absent or Inactive/Terminated.
- An active interval ends at the next observation whose value is
  Inactive/Terminated, or NULL if no such observation exists.
- A child with NO `status` observations **MUST** be treated as
  always-active (synthetic [-infinity, NULL) interval) so connectors
  that emit `parent_email` without `status` do not silently drop
  every edge.
- Re-activation (Inactive -> Active) **MUST** produce a second
  `org_chart` row for the same (child, parent, source)
  rather than reopening the existing closed row; SCD2 history
  reflects every deactivation/reactivation cycle honestly.

The seeder **MUST** process BambooHR accounts ahead of other source
types in step 5 (person_id assignment) so the canonical
`supervisorEmail` source establishes `person_id`s before downstream
connectors share emails with it.

After the swap, the seeder **MUST** count two-hop cycles among
CURRENT edges (`valid_to IS NULL`) — pairs `(A->B)` and `(B->A)` on
the same `(tenant, source_type, source_id)` — and **MUST** emit a
WARN line when the count is non-zero, without failing the pipeline.
Deeper cycles (A->B->C->A) are not detected in Phase 1; the Phase-3
`/v1/subchart/{person_id}?depth=N` recursive CTE bounds traversal
by `depth` to make those harmless to consumers.

**Rationale**: Append-only `persons` with latest-per-partition makes
the rebuild deterministic; symmetry with `account_person_map` lets
operators reason about both caches the same way. The two-source
union keeps the cache useful today (Source 2) while making the
future reconciliation path activate transparently (Source 1). No
stubs avoids inheriting the deferred operator-resolution work from
ADR-0002.

**Actors**: `cpt-insightspec-actor-seed-pipeline`,
`cpt-insightspec-actor-mariadb`

#### Read current parent and children edges

- [x] `p1` - **ID**: `cpt-insightspec-fr-identity-org-chart-read`

The service **MUST** expose `IPersonsReader.GetCurrentParentsAsync`
and `GetCurrentChildrenAsync` returning `OrgChartEdge` records
scoped to a single tenant. Both **MUST** read CURRENT edges only
(`valid_to IS NULL`) and **MUST** preserve per-source-instance edge
granularity in the result. Temporal "as-of T" queries are Phase 3+
and add a new method on the same interface; the table's
`idx_valid_from` is the supporting index.

**Rationale**: Phase-2 endpoint enrichment (parent/subordinates
fields on `/v1/persons` and `/v1/profiles`) and Phase-3 subchart
recursion both call the same two reads — keeping them on
`IPersonsReader` keeps the abstraction stable across the three
phases.

**Actors**: `cpt-insightspec-actor-api-gateway`,
`cpt-insightspec-actor-mariadb`

## 6. Non-Functional Requirements

### 6.1 NFR Inclusions

#### P95 lookup latency

- [x] `p1` - **ID**: `cpt-insightspec-nfr-identity-latency`

The system **MUST** answer `GET /v1/persons/{email}` within
**50 ms p95** for tenants with under 50 000 persons.

**Threshold**: p95 ≤ 50 ms measured at the api-gateway → identity
hop; tenants with > 50 000 persons fall under the project default
(p95 ≤ 200 ms).

**Rationale**: Single-row cardinality on a covered index
(`idx_value_id`) makes this achievable without caching; the bound is
tight to keep gateway-side timeouts conservative.

#### Memory budget without caching

- [x] `p1` - **ID**: `cpt-insightspec-nfr-identity-memory`

The system **MUST** stay under **384 MiB RSS** at steady state with
zero in-memory full-table cache.

**Threshold**: RSS ≤ 384 MiB across a 24 h soak with 100 RPS mixed
hot/cold reads against a 50 000-row dataset.

**Rationale**: Architecture decision (ADR-0002): no in-memory cache,
every read hits MariaDB; the memory budget reflects that.

#### Structured JSON logs with PII redaction

- [x] `p1` - **ID**: `cpt-insightspec-nfr-identity-logging-pii`

The system **MUST** emit structured JSON logs via Serilog
`CompactJsonFormatter` with the enricher `service=identity`.
Request-logging middleware **MUST** record only an allow-listed
property set (`RequestMethod`, `RequestPath` template, `StatusCode`,
`Elapsed`, `RequestId`, `ConnectionId`, `@tr`/`@sp` trace+span IDs)
and **MUST** redact the raw email path segment to
`/v1/persons/<redacted>`. Unhandled-exception payloads **MUST**
include exception type + message + sanitised `db_target`
(`host:port/db`, no credentials) and **MUST NOT** include the
connection string.

**Threshold**: Manual log-scrape audit shows zero raw emails in
captured request paths across the test suite.

**Rationale**: Emails are PII; the URL template carries the customer's
mailbox locally — leaking it into log aggregation would breach the
project-wide PII handling policy.

#### `BINARY(16)` UUID round-trip

- [x] `p1` - **ID**: `cpt-insightspec-nfr-identity-uuid-roundtrip`

All UUIDs (`insight_tenant_id`, `insight_source_id`, `person_id`,
`author_person_id`) **MUST** round-trip as `BINARY(16)` via
`Guid.ToByteArray()` / `new Guid(byte[])`. The repository
**MUST NOT** rely on MySqlConnector's default `ToString()` fallback,
which produces a 36-char form that the `BINARY(16)` column silently
truncates to 16 ASCII bytes.

**Threshold**: An integration test seeds a row by bytes and reads it
back by Guid; equality holds byte-for-byte.

**Rationale**: The truncation bug was caught in the Python seeder and
is the canonical UUID-handling failure mode for MariaDB clients;
this NFR forces the explicit bytes binding everywhere.

### 6.2 NFR Exclusions

- **High availability via in-memory replication**: not applicable —
  the service is stateless beyond its connection pool; HA is
  addressed by Kubernetes `replicaCount` and MariaDB's own
  replication, not by service-level state replication.
- **Write throughput SLO**: not applicable — service is read-only;
  write paths (seed, reconciliation) carry their own SLOs.

## 7. Public Library Interfaces

### 7.1 Public API Surface

#### `GET /v1/persons/{email}` — Person lookup

- [x] `p1` - **ID**: `cpt-insightspec-interface-identity-person-lookup`

**Type**: HTTP/REST endpoint.

**Stability**: stable for Phase 1 contract; Phase 2 will add a POST
counterpart accepting `{id, id_type}` without breaking this GET.

**Description**: Resolves `{email}` to a single `PersonResponse`
JSON body. Comparison is case-insensitive at the storage layer
(ADR-0011). Tenant supplied via `X-Insight-Tenant-Id` header
(preferred) or config default.
Returns 200 + body on hit, 404 + RFC 7807 on miss, 400 + RFC 7807
on missing tenant, 5xx on service error.

**Breaking Change Policy**: Major version bump for response-shape
changes; additive fields are non-breaking; the URL template is
stable across minor versions.

#### `GET /health` — Database readiness

- [x] `p1` - **ID**: `cpt-insightspec-interface-identity-health`

**Type**: HTTP/REST endpoint.

**Stability**: stable.

**Description**: Pings MariaDB. Returns 200 when the pool is
healthy, 503 otherwise. Wired as the Helm readiness probe.

**Breaking Change Policy**: No payload shape — never breaking.

#### `GET /healthz` — Process liveness

- [x] `p1` - **ID**: `cpt-insightspec-interface-identity-healthz`

**Type**: HTTP/REST endpoint.

**Stability**: stable.

**Description**: Returns 200 `text/plain "ok"` if the process is up;
does not touch MariaDB. Wired as the Helm liveness probe.

**Breaking Change Policy**: No payload shape — never breaking.

### 7.2 External Integration Contracts

#### `IDENTITY__*` env-var contract

- [x] `p1` - **ID**: `cpt-insightspec-contract-identity-env-config`

**Direction**: required from operator (Helm umbrella or BYO Secret).

**Protocol/Format**: ASP.NET Core configuration with double-underscore
section delimiter (`IDENTITY__mariadb__url`,
`IDENTITY__identity__tenant_default_id`, etc.). YAML overlay supported
via `appsettings.yaml`.

**Compatibility**: Backward-compatible field additions only; renames
require a major version bump of the chart's umbrella schema.

#### `insight-identity-config` Secret

- [x] `p2` - **ID**: `cpt-insightspec-contract-identity-config-secret`

**Direction**: provided by umbrella chart, consumed by the service
pod via `envFrom`.

**Protocol/Format**: Kubernetes `Secret` (string data) containing
the `IDENTITY__*` keys. URL form preferred
(`IDENTITY__mariadb__url`); MySqlConnector KV form supported
(`IDENTITY__mariadb__connection_string`) for callers needing options
the URL shape cannot express.

**Compatibility**: Stable across chart minor versions; additive fields
non-breaking.

## 8. Use Cases

#### Resolve email to person

- [x] `p1` - **ID**: `cpt-insightspec-usecase-identity-lookup-email`

**Actor**: `cpt-insightspec-actor-api-gateway`

**Preconditions**:
- Seed pipeline has populated at least one `value_type='email'`
  observation for the target tenant.
- Caller's request carries `X-Insight-Tenant-Id` or the service is
  configured with a `tenant_default_id`.

**Main Flow**:
1. api-gateway receives an analytics request that needs person
   enrichment.
2. api-gateway resolves the JWT principal and derives the email +
   tenant header.
3. api-gateway issues `GET /v1/persons/{email}` to the service.
4. The service resolves the `person_id` via the latest-per-source
   email observation (case-insensitive comparison per ADR-0011),
   hydrates all attributes, and returns 200 + JSON.
5. api-gateway merges the person object into the analytics response.

**Postconditions**:
- The analytics response carries the resolved person attributes.

**Alternative Flows**:
- **No observation matches**: service returns 404 + RFC 7807 problem
  details; api-gateway includes a `person_unresolved` flag in the
  analytics response.
- **No tenant**: service returns 400 +
  `urn:insight:error:tenant_unresolved`; api-gateway returns 401 to
  the original caller (the missing tenant means the principal was
  not properly resolved).

#### Liveness and readiness

- [x] `p1` - **ID**: `cpt-insightspec-usecase-identity-probes`

**Actor**: `cpt-insightspec-actor-platform-sre`

**Preconditions**:
- Pod is scheduled with the Helm probe wiring.

**Main Flow**:
1. kubelet hits `/healthz` every 10 s for liveness.
2. kubelet hits `/health` every 5 s for readiness.
3. A failing `/health` (DB unreachable) flips the pod out of the
   Service endpoints until the pool recovers.

**Postconditions**:
- Traffic is routed only to pods whose DB pool is healthy.

## 9. Acceptance Criteria

- [ ] An integration test against a Testcontainers MariaDB returns
      the seeded Alice record with email, display_name, job_title
      fields populated.
- [ ] The same integration test returns 404 + RFC 7807 body for an
      unknown email.
- [ ] The same integration test returns 400 +
      `urn:insight:error:tenant_unresolved` when the request omits
      the tenant header and no default is configured.
- [ ] `dotnet test` passes for both unit and integration projects on
      a fresh checkout.
- [ ] Helm template renders `Service`, `Deployment`, `Secret`, and
      `_helpers.tpl` host references with the canonical
      `insight-identity` name.
- [ ] DbUp creates `persons` and `account_person_map` against an
      empty `identity` MariaDB on first pod start; re-running the
      pod is a no-op against `SchemaVersions`.
- [ ] `cypilot validate --skip-code --artifact docs/components/backend/identity-resolution/identity`
      reports zero errors.

## 10. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| MariaDB `identity` database | Read target + DbUp migration target. | p1 |
| Seed pipeline (`seed-persons-from-identity-input.py`) | Populates the rows the reader returns. | p1 |
| BambooHR `bamboohr__identity_inputs` dbt model | Source of identity observations for the first connector to land on the new schema. | p1 |
| Reconciliation service (future) | Writes `parent_person_id` observations consumed by Phase 2 org-tree expansion. | p2 |
| api-gateway | Sole external caller in Phase 1. | p1 |

## 11. Assumptions

- Single MariaDB database per service instance — no sharding, no
  multi-region writes from this service.
- The seed pipeline's `INSERT IGNORE` semantics guarantee no duplicate
  observations under the natural-key UNIQUE; the reader does not
  deduplicate beyond `ROW_NUMBER()` filtering.
- `insight_tenant_id` is a `BINARY(16)` UUID for the lifetime of this
  service; if the project adopts string tenants the schema (and this
  PRD) need a major revision.
- All callers in the Insight platform forward via api-gateway —
  external direct callers are out of scope until OIDC subject mapping
  ships.

## 12. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| `persons` schema evolves under us | Reader SQL drifts and silently returns wrong fields. | Centralise SQL in `Insight.Identity.Infrastructure/MariaDb/Sql.cs`; integration tests pin column names. |
| Misconfigured `tenant_default_id` in multi-tenant cluster | Wrong-tenant data leaks to a header-less caller. | Composite resolver always lets header win; helm validator warns when `tenantDefaultId` is set with `identity.deploy=true` in a production overlay (planned). |
| Seed pipeline never runs on a fresh cluster | Every lookup returns 404 indefinitely. | `/health` only checks DB reachability — the operator sees green pods and an empty `persons` table; document the post-install seed step in the README. |
| BambooHR connector evolves the `value_type` set | New observations are silently ignored. | Hardcoded routing in ADR-0007 + integration test that asserts the projection of each known `value_type`. |
