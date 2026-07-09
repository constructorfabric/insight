---
status: proposed
date: 2026-04-03
---

# ADR-0002: Database Field Naming and Type Conventions

- [ ] `p1` - **ID**: `cpt-insightspec-adr-db-field-conventions`

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Decisions](#decisions)
  - [D1: Temporal Range Naming -- effective_from / effective_to](#d1-temporal-range-naming----effectivefrom--effectiveto)
  - [D2: Tenant Identifier -- insight_tenant_id UUID](#d2-tenant-identifier----insighttenantid-uuid)
  - [D3: Actor Attribution -- UUID Foreign Key](#d3-actor-attribution----uuid-foreign-key)
  - [D4: MariaDB Timestamp Type -- DATETIME(3)](#d4-mariadb-timestamp-type----datetime3)
  - [D5: ClickHouse Enum Strategy -- LowCardinality(String)](#d5-clickhouse-enum-strategy----lowcardinalitystring)
  - [D6: ClickHouse Nullable Avoidance](#d6-clickhouse-nullable-avoidance)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

## Context and Problem Statement

Insight has 8 microservices writing to MariaDB and ClickHouse. Early specs (PR #49 backend, PR #54 identity-resolution) introduced inconsistent conventions for the same concepts: different names for temporal ranges, different types for `tenant_id`, different patterns for audit attribution. Without a shared standard, each new service will invent its own column names and types, making cross-service queries, event correlation, and onboarding harder over time. Which conventions should all services follow?

## Decision Drivers

* Consistency across 8+ services -- a developer moving between services should recognise column semantics instantly by name
* Existing project decisions -- the [API Guideline](../../api-guideline/API.md) already mandates `snake_case`, `created_at`/`updated_at`, ISO-8601 UTC timestamps
* Conflict resolution -- PR #49 and PR #54 diverge on temporal naming, tenant_id type, and actor attribution; one convention must win
* Database engine best practices -- ClickHouse and MariaDB have documented recommendations for types and patterns
* Joinability -- audit logs, events, and API responses reference the same entities; types must be compatible

## Considered Options

Each decision below lists its specific alternatives. This ADR bundles six related naming/type conventions because they are interdependent (e.g., choosing UUID for tenant_id depends on choosing UUID for PKs) and too granular for individual ADRs.

## Decision Outcome

Adopt the six conventions documented below as mandatory for all internal Insight tables. See the full specification in the [Database Field Naming & Type Conventions](../README.md) document.

### Consequences

* Good, because all services use identical column names and types for the same concepts
* Good, because resolves all known conflicts between PR #49 and PR #54
* Good, because aligns with existing API Guideline and ClickHouse/MariaDB best practices
* Good, because new services can follow the convention document without reading every existing schema
* Bad, because PR #54 (identity-resolution DESIGN) requires updates to comply -- `valid_from/valid_to` renamed, `tenant_id` → `insight_tenant_id`, `performed_by` replaced with UUID FK
* Bad, because existing DESIGN documents across the project (backend, ingestion, connector, individual connectors) require `tenant_id` → `insight_tenant_id` renaming -- see [Known Convention Violations](../README.md#15-known-convention-violations)
* Bad, because conventions add constraints that may feel rigid for edge cases -- exceptions must be documented

### Confirmation

* Code review: all `CREATE TABLE` statements in DESIGN documents use the mandated column names and types
* `cfs validate` checks cross-reference consistency for traceability IDs
* Grep for anti-patterns: `valid_from`, `owned_from`, `performed_by VARCHAR`, `TIMESTAMP` (MariaDB), `Enum8`, `Nullable(String)` (ClickHouse)
* Each service's integration tests use real databases (testcontainers) -- schema mismatches surface as test failures

## Decisions

### D1: Temporal Range Naming -- effective_from / effective_to

**Problem**: Three competing naming patterns exist across specs.

| Option | Used in | Convention |
|--------|---------|------------|
| `effective_from` / `effective_to` | Backend DESIGN (PR #49) | Majority (7 services) |
| `valid_from` / `valid_to` | Identity Resolution DESIGN (PR #54) | 1 service |
| `owned_from` / `owned_until` | Identity Resolution alias table (PR #54) | 1 table |

**Decision**: Use `effective_from` / `effective_to` everywhere.

**Rationale**: Backend DESIGN (PR #49) covers 7 of 8 services and establishes the majority convention. One consistent pair eliminates ambiguity. The suffix `_to` (not `_until`) matches the `_from` / `_to` symmetry.

**Rules**:
- All temporal ranges use half-open intervals: `[effective_from, effective_to)`
- `effective_to IS NULL` means "currently active"
- Use `DATE` when business granularity is days; `DATETIME(3)` when sub-day precision is needed
- Never use `BETWEEN` for temporal queries

### D2: Tenant Identifier -- insight_tenant_id UUID

**Problem**: PR #49 uses `tenant_id UUID`; PR #54 uses `tenant_id VARCHAR(100)`. Additionally, external systems (Azure, Salesforce) use `tenant_id` as their own field name, creating ambiguity.

| Option | Type | Collision risk | Consistency |
|--------|------|---------------|-------------|
| `insight_tenant_id UUID` | UUID (16 bytes) | None -- `insight_` prefix is unambiguous | Matches connector config convention |
| `tenant_id UUID` | UUID (16 bytes) | Collides with Azure `tenant_id`, etc. | Breaks in Bronze/Silver context |
| `tenant_id VARCHAR(100)` | VARCHAR (up to 100 bytes) | Same collision + type inconsistency | Breaks UUID-everywhere convention |

**Decision**: `insight_tenant_id` is always `UUID NOT NULL`. See [ADR-0003](0003-insight-prefixed-tenant-id.md) for full rationale.

**Rationale**: The `insight_` prefix eliminates name collisions with source systems, aligns with the existing connector config convention (`insight_tenant_id`, `insight_source_id`), and the UUID type is consistent with ADR-0001.

### D3: Actor Attribution -- UUID Foreign Key

**Problem**: PR #49 uses `actor_person_id UUID` (FK to persons.id) for audit attribution; PR #54 uses `performed_by VARCHAR(100)` storing a username string.

| Option | Joinability | Resilience to rename | Storage |
|--------|-------------|---------------------|---------|
| `actor_person_id UUID` (FK) | Direct JOIN to persons | Immune -- UUID is stable | 16 bytes |
| `performed_by VARCHAR(100)` | Requires lookup by username | Breaks if username changes | Up to 100 bytes |

**Decision**: Always use UUID FK to `person.id` for actor attribution.

**Rationale**: Usernames and emails change. A UUID FK is stable, joinable, and consistent with the all-UUID convention. Column name follows the `{role}_{entity}_id` pattern: `actor_person_id`, `granted_by`, `resolved_by` (all UUID FKs to persons.id).

**Naming**:
- `actor_person_id` -- who performed the action (audit events)
- `granted_by` -- who granted a role/permission
- `resolved_by` -- who resolved a conflict/alert

### D4: MariaDB Timestamp Type -- DATETIME(3)

**Problem**: MariaDB offers both `TIMESTAMP` and `DATETIME`. The API Guideline mandates ISO-8601 with milliseconds (`.SSS`).

| Option | Precision | Range | Timezone behaviour | Size |
|--------|-----------|-------|--------------------|------|
| `TIMESTAMP` | seconds (or fsp) | 1970-2038 | Implicit UTC conversion | 4 bytes |
| `DATETIME` | seconds (or fsp) | 1000-9999 | Stored as-is, no conversion | 8 bytes |
| `DATETIME(3)` | milliseconds | 1000-9999 | Stored as-is, no conversion | 8 bytes |

**Decision**: Use `DATETIME(3)` for all timestamp columns in MariaDB.

**Rationale**:
- Millisecond precision matches the API's ISO-8601 `.SSS` format -- no precision loss on round-trip
- No 2038 problem (TIMESTAMP wraps; DATETIME does not)
- No implicit timezone conversion -- application controls UTC explicitly
- All `DATETIME(3)` values **MUST** be stored in UTC -- this is an application-level responsibility since `DATETIME` has no built-in timezone semantics. MariaDB connection string should include `SET time_zone = '+00:00'` to ensure `CURRENT_TIMESTAMP` and `NOW()` return UTC
- Standard defaults: `DEFAULT CURRENT_TIMESTAMP(3)` and `ON UPDATE CURRENT_TIMESTAMP(3)` for `created_at`/`updated_at`

### D5: ClickHouse Enum Strategy -- LowCardinality(String)

**Problem**: ClickHouse offers `Enum8`/`Enum16` and `LowCardinality(String)` for categorical columns.

| Option | Schema evolution | Storage | Query syntax |
|--------|-----------------|---------|-------------|
| `Enum8('a'=1, 'b'=2)` | `ALTER TABLE` required to add values | Compact (1-2 bytes) | Must use exact string values |
| `LowCardinality(String)` | No schema change needed | Dictionary-encoded, similarly compact for <10K values | Standard string comparison |

**Decision**: Use `LowCardinality(String)` for all categorical/enum columns in ClickHouse.

**Rationale**: Adding a new status value or action type should not require a ClickHouse `ALTER TABLE`. `LowCardinality(String)` achieves comparable compression via dictionary encoding for columns with fewer than ~10,000 distinct values (all Insight categorical columns qualify). This is the approach recommended by ClickHouse documentation.

### D6: ClickHouse Nullable Avoidance

**Problem**: ClickHouse stores an additional `UInt8` null-mask column for every `Nullable` column, adding storage and processing overhead.

| Option | Storage overhead | Query complexity |
|--------|-----------------|-----------------|
| `Nullable(T)` | +1 byte per row per column | `IS NULL` checks in every query |
| Default/sentinel values | None | Simpler predicates |

**Decision**: Avoid `Nullable` in ClickHouse unless null carries distinct semantic meaning that cannot be represented by a sentinel value.

**Rationale**: ClickHouse best practice. Use empty string `''` for text, `0` for counts, `'1970-01-01'` for dates, `toUUID('00000000-...')` for UUIDs. Reserve `Nullable` for cases where "unknown/not applicable" is semantically different from "empty" (e.g., `Nullable(UUID)` for optional FK references where the zero UUID would be misleading).

## More Information

The full specification with examples and anti-patterns is in [Database Field Naming & Type Conventions](../README.md).

**Existing project standards referenced**:
- [API Guideline](../../api-guideline/API.md) -- `snake_case`, `created_at`/`updated_at`, ISO-8601 UTC `.SSS`, UUIDv7
- [ADR-0001: UUIDv7 Primary Key](0001-uuidv7-primary-key.md) -- UUID-only PK strategy

**ClickHouse best practices**:
- [Avoid Nullable Columns](https://clickhouse.com/docs/en/cloud/bestpractices/avoid-nullable-columns)
- [LowCardinality Optimization](https://clickhouse.com/docs/en/sql-reference/data-types/lowcardinality)

**MariaDB documentation**:
- [DATETIME vs TIMESTAMP](https://mariadb.com/docs/server/reference/data-types/date-and-time-data-types/timestamp/)
- [UUID Data Type](https://mariadb.com/docs/server/reference/data-types/string-data-types/uuid-data-type/)

## Traceability

This decision directly addresses the following requirements or design elements:

* `cpt-insightspec-nfr-be-tenant-isolation` -- D2 (UUID tenant_id) ensures consistent tenant isolation type across all storage systems
* `cpt-insightspec-fr-be-audit-trail` -- D3 (UUID actor attribution) ensures audit events can be joined to person records without fragile string matching
* `cpt-insightspec-fr-be-visibility-policy` -- D1 (effective_from/effective_to) standardises the temporal range fields that implement follow-the-unit-strict policy
* `cpt-insightspec-fr-be-org-tree-sync` -- D1 (effective_from/effective_to) applies to person_org_membership temporal fields
* `cpt-insightspec-principle-be-secure-by-default` -- D4 (DATETIME(3)) avoids implicit timezone conversions that could cause time-based access control errors
* `cpt-insightspec-fr-be-forward-only-migrations` -- D5 (LowCardinality over Enum) avoids ALTER TABLE for ClickHouse enum evolution, simplifying forward-only migrations
