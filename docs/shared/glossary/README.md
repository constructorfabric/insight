# Database Field Naming & Type Conventions

> Applies to all **internal Insight tables** (MariaDB metadata, ClickHouse analytics/audit).
> Does NOT apply to Bronze tables ingested from external sources via Airbyte connectors — those preserve source-native schemas.

This document defines mandatory naming patterns and data types for columns that appear across multiple services and layers. The goal is consistency: any engineer reading any table in any service should immediately recognise the role of a column by its name.

## Architecture Decision Records

| ADR | Decision | Status |
|-----|----------|--------|
| [ADR-0001](ADR/0001-uuidv7-primary-key.md) | UUIDv7 as universal primary key -- single UUID PK per table, no INT surrogates | proposed |
| [ADR-0002](ADR/0002-database-field-conventions.md) | Database field naming and type conventions -- temporal naming, tenant_id type, actor attribution, DATETIME(3), ClickHouse patterns | proposed |
| [ADR-0003](ADR/0003-insight-prefixed-tenant-id.md) | Use `insight_tenant_id` instead of `tenant_id` -- avoid name collision with source systems | proposed |

---

<!-- toc -->

- [Architecture Decision Records](#architecture-decision-records)
- [1. General Rules](#1-general-rules)
- [2. Identifiers & Primary Keys](#2-identifiers--primary-keys)
  - [2.1 Internal Entity Tables](#21-internal-entity-tables)
  - [2.2 Why UUID-Only, Not INT Surrogate + UUID](#22-why-uuid-only-not-int-surrogate--uuid)
- [3. Tenant & Source Isolation](#3-tenant--source-isolation)
  - [3.1 Tenant Identifier -- insight_tenant_id](#31-tenant-identifier----insighttenantid)
  - [3.2 Source Tracking Fields](#32-source-tracking-fields)
- [4. Timestamp Fields](#4-timestamp-fields)
  - [4.1 Record Lifecycle Timestamps](#41-record-lifecycle-timestamps)
  - [4.2 Temporal Validity (Effective Ranges)](#42-temporal-validity-effective-ranges)
  - [4.3 Job / Processing Timestamps](#43-job--processing-timestamps)
  - [4.4 Event Timestamps](#44-event-timestamps)
- [5. Foreign Key References](#5-foreign-key-references)
- [6. ClickHouse-Specific Conventions](#6-clickhouse-specific-conventions)
  - [ORDER BY Key Design](#order-by-key-design)
  - [Nullable](#nullable)
  - [LowCardinality](#lowcardinality)
  - [Partitioning](#partitioning)
  - [TTL](#ttl)
- [7. MariaDB-Specific Conventions](#7-mariadb-specific-conventions)
  - [UUID Type](#uuid-type)
  - [DATETIME Precision](#datetime-precision)
  - [Timezone](#timezone)
  - [Character Sets](#character-sets)
- [8. Boolean Fields](#8-boolean-fields)
- [9. Soft-Delete](#9-soft-delete)
- [10. Observation Timestamps](#10-observation-timestamps)
- [11. String Length Tiers](#11-string-length-tiers)
- [12. Anti-Patterns](#12-anti-patterns)
- [13. Proposals](#13-proposals)
  - [P1: Hierarchy & Tree Fields](#p1-hierarchy--tree-fields)
  - [P2: Status & Enum Fields](#p2-status--enum-fields)
  - [P3: Actor / Audit Attribution](#p3-actor--audit-attribution)
  - [P4: Confidence & Scoring Fields](#p4-confidence--scoring-fields)
  - [P5: JSON / Flexible Storage](#p5-json--flexible-storage)
  - [P6: Hash & Change Detection](#p6-hash--change-detection)
  - [P7: Business Temporal Validity in MariaDB](#p7-business-temporal-validity-in-mariadb)
- [14. Proposals with Known Contradictions](#14-proposals-with-known-contradictions)
  - [SCD Type 2 in MariaDB](#scd-type-2-in-mariadb)
  - [MariaDB Partial Index Workaround](#mariadb-partial-index-workaround)
- [15. Known Convention Violations](#15-known-convention-violations)
  - [Code](#code)
  - [Spec documents](#spec-documents)

<!-- /toc -->

---

## 1. General Rules

| Rule | Convention | Source |
|------|-----------|--------|
| Column naming | `snake_case`, lowercase | [API Guideline](../api-guideline/API.md) §4 |
| Table naming | `snake_case`, lowercase, **plural** (`persons`, `alert_rules`, `org_units`) | Project convention |
| ID format | UUIDv7 (time-ordered) | [API Guideline](../api-guideline/README.md) §3 |
| Timestamp format | ISO-8601 UTC with milliseconds in JSON; DB-native types in storage | [API Guideline](../api-guideline/API.md) §3 |
| Nullability | Avoid unless null carries distinct semantic meaning | ClickHouse best practice; MariaDB convention |

---

## 2. Identifiers & Primary Keys

### 2.1 Internal Entity Tables

Every **internal entity table** (MariaDB or ClickHouse) has a single `id` column as primary key or unique identifier:

**MariaDB:**

```sql
id UUID NOT NULL DEFAULT uuid_v7() PRIMARY KEY
```

> MariaDB 10.7+ native `UUID` type stores 16 bytes internally. Always use `uuid_v7()` (time-ordered) to preserve insert ordering and minimise InnoDB page splits.

**ClickHouse (internal tables, e.g., audit events):**

```sql
id UUID DEFAULT generateUUIDv7()
```

> In ClickHouse `id` is not a PK in the RDBMS sense — it serves as a unique row identifier for filtering and joining. It should appear **last** in the `ORDER BY` key (if at all). See [section 6](#6-clickhouse-specific-conventions).

**This applies to**: all tables that represent Insight's own domain entities and internal data — `persons`, `aliases`, `org_units`, `alert_rules`, `dashboards`, `metrics`, `connector_configs`, `tenant_keys`, `secrets`, audit events, email delivery logs, etc. This includes both MariaDB and ClickHouse tables owned by Insight services.

**This does NOT apply to**:
- **Bronze tables** — raw data from external sources via Airbyte; schema is source-native, no UUID PK
- **Silver tables** — unified analytical tables in ClickHouse (`class_commits`, `class_people`, etc.); use composite `ORDER BY` keys, not UUID PKs
- **Gold tables** — aggregated metrics in ClickHouse; same as Silver

**Foreign key references** use the pattern `{entity}_id`:

```sql
person_id         UUID    -- FK to persons.id
org_unit_id       UUID    -- FK to org_units.id
metric_id         UUID    -- FK to metrics.id
insight_tenant_id UUID    -- FK to tenants.id (see section 3)
```

### 2.2 Why UUID-Only, Not INT Surrogate + UUID

The identity-resolution DESIGN (PR #54) proposed `id INT AUTO_INCREMENT` as PK with a separate `person_id UUID` column. We standardise on **UUID-only** for the following reasons:

| Factor | INT surrogate + UUID | UUID-only (UUIDv7) |
|--------|---------------------|---------------------|
| Schema complexity | Two ID columns to manage, two indexes | One column, one index |
| Cross-service references | Services must know both IDs or always join | Single ID works everywhere (API, DB, events, logs) |
| InnoDB page fill (UUIDv7) | ~94% (sequential INT) | ~90% (time-ordered, minor random suffix) |
| Secondary index overhead | 4 bytes per entry | 16 bytes per entry |
| Practical impact at Insight scale | Negligible — metadata tables, not billions of rows | Negligible |
| API consistency | API exposes UUID, DB uses INT — mapping layer needed | API and DB use the same value |

**Decision**: the marginal InnoDB performance benefit of INT surrogates does not justify the complexity for Insight's metadata workloads (thousands to low millions of rows per tenant). UUIDv7 provides near-sequential ordering. All services, events, logs, and APIs use one identifier per entity.

**Exception**: ClickHouse Bronze/Silver tables do NOT use UUID PKs — they use composite `ORDER BY` keys optimised for analytical queries. See [section 6](#6-clickhouse-specific-conventions).

---

## 3. Tenant & Source Isolation

### 3.1 Tenant Identifier -- insight_tenant_id

Every table in every storage system includes `insight_tenant_id`:

| Storage | Column | Type | Enforcement |
|---------|--------|------|-------------|
| MariaDB | `insight_tenant_id` | `UUID NOT NULL` | `SecureConn` + `AccessScope` (modkit-db) |
| ClickHouse | `insight_tenant_id` | `UUID` | Row-level filter on all queries |
| Redis | key prefix | `{insight_tenant_id}:` | Application convention |
| Redpanda | message field | UUID (JSON) | Consumer-side filter |
| S3 / MinIO | object prefix | `{insight_tenant_id}/` | Application convention |

**Why `insight_tenant_id`, not `tenant_id`?** Many external systems use `tenant_id` as their own field name (e.g., Azure `tenant_id`, Salesforce `org_id`). The `insight_` prefix eliminates ambiguity across all layers -- Bronze tables, Silver tables, internal metadata, connector configs, and API payloads. One name, zero collisions. See [ADR-0003](ADR/0003-insight-prefixed-tenant-id.md).

`insight_tenant_id` is always `UUID`, consistent with the project-wide ID convention. Never `VARCHAR` or `String`.

### 3.2 Source Tracking Fields

Tables that contain or reference data from external source systems include source tracking fields:

| Column | Type | Required | Description |
|--------|------|----------|-------------|
| `insight_source_id` | `UUID NOT NULL` | When source-specific | Connector instance identifier -- distinguishes between two GitLab instances for the same tenant. Propagated from Bronze layer |
| `insight_source_type` | `VARCHAR(100) NOT NULL` | When source-specific | Source system type (e.g., `github`, `gitlab`, `bamboohr`, `jira`, `slack`). Replaces `source_system` / `source` |
| `source_account_id` | `VARCHAR(500) NOT NULL` | When source-specific | Unique account/user ID within the source system (e.g., GitHub user ID, Jira account ID). Source-native format, not normalised |

**Rules:**
- `insight_source_id` and `insight_source_type` always appear together -- a source type without an instance ID is ambiguous when a tenant has multiple instances of the same system
- `source_account_id` is the raw identifier from the external system -- not a UUID, not normalised. Format varies by source
- These fields are NOT present on purely internal tables (e.g., `alert_rules`, `dashboards`, `user_roles`) that have no relationship to external source data

**Naming convention:** `insight_` prefix for platform-injected fields; no prefix for source-native fields (`source_account_id`).

---

## 4. Timestamp Fields

### 4.1 Record Lifecycle Timestamps

Present on virtually every MariaDB table:

| Column | Type (MariaDB) | Nullable | Default | Description |
|--------|----------------|----------|---------|-------------|
| `created_at` | `DATETIME(3) NOT NULL` | No | `CURRENT_TIMESTAMP(3)` | When the record was first inserted |
| `updated_at` | `DATETIME(3) NOT NULL` | No | `CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3)` | When the record was last modified |

- Always `DATETIME(3)` (millisecond precision) to match the API's ISO-8601 `.SSS` format.
- `DATETIME` over `TIMESTAMP` — wider range (no 2038 problem), no implicit timezone conversion, predictable behaviour.
- All `DATETIME(3)` values are stored in **UTC**. MariaDB `DATETIME` has no timezone semantics — application code is responsible for converting to UTC before write and from UTC after read. Set `time_zone = '+00:00'` in the MariaDB connection string to ensure `CURRENT_TIMESTAMP` and `NOW()` return UTC.
- `created_at` is immutable after insert.

### 4.2 Temporal Validity (Effective Ranges)

For records with a period of validity (org memberships, SCD2 versions, alias ownership):

| Column | Type (MariaDB) | Nullable | Description |
|--------|----------------|----------|-------------|
| `effective_from` | `DATE NOT NULL` | No | Start of validity (inclusive) |
| `effective_to` | `DATE NULL` | Yes | End of validity (exclusive); `NULL` = currently active |

**Rules:**
- All temporal ranges use **half-open intervals**: `[effective_from, effective_to)`
- `NULL` in `effective_to` means "currently active / no known end"
- Never use `BETWEEN` for temporal queries — use `effective_from <= @date AND (effective_to IS NULL OR effective_to > @date)`
- Use `DATE` (not `DATETIME`) when the business granularity is days (org memberships, role assignments)
- Use `DATETIME(3)` when sub-day precision matters (SCD2 version ranges in identity resolution)

**Naming convention**: always `effective_from` / `effective_to`. Not `valid_from/valid_to`, not `owned_from/owned_until` — one consistent pair across all services.

### 4.3 Job / Processing Timestamps

For records that are periodically re-evaluated by background jobs:

| Column | Type (MariaDB) | Nullable | Description |
|--------|----------------|----------|-------------|
| `last_analyzed_at` | `DATETIME(3) NULL` | Yes | When a job last processed this record (e.g., bootstrap, identity resolution) |
| `resolved_at` | `DATETIME(3) NULL` | Yes | When a conflict / unmapped alias was resolved |

`NULL` means "never processed" or "never resolved".

### 4.4 Event Timestamps

For ClickHouse event tables (audit log, analytics):

| Column | Type (ClickHouse) | Description |
|--------|-------------------|-------------|
| `timestamp` | `DateTime64(3, 'UTC')` | When the event occurred |

- Use `DateTime64(3, 'UTC')` (millisecond precision) for event tables where sub-second ordering matters.
- Use `DateTime` (second precision) for analytical aggregates where milliseconds add no value.
- Always specify `'UTC'` timezone explicitly.

---

## 5. Foreign Key References

**General pattern:** `{referenced_entity}_id UUID`

**Rules:**
- FK column name matches the referenced table's logical entity: `person_id`, `org_unit_id`, `metric_id`
- `insight_tenant_id` is a special case (see [section 3](#3-tenant--source-isolation))
- When the same entity is referenced twice in one table, prefix with role: `source_person_id`, `target_person_id`, `manager_person_id`
- Self-referential FK for hierarchies: `parent_id UUID NULL` — `NULL` means root node. Application must prevent circular references. Materialised path fields (`path`, `depth`) are proposed in [section 13](#13-proposals)

Concrete FK lists per domain will be defined in corresponding DESIGN documents.

---

## 6. ClickHouse-Specific Conventions

### ORDER BY Key Design

```sql
ORDER BY (insight_tenant_id, event_date, entity_type, id)
```

- **First**: `insight_tenant_id` — always filtered, lowest cardinality
- **Middle**: date/category columns — filtered frequently, medium cardinality
- **Last** (if needed): UUID — highest cardinality, only for deduplication

Never place UUID first — it destroys granule-level data skipping and compression.

### Nullable

Avoid `Nullable` unless null is semantically meaningful. Prefer:
- Empty string `''` for text
- `0` for counts
- Sentinel `'1970-01-01'` for dates
- `toUUID('00000000-0000-0000-0000-000000000000')` for UUIDs

Each `Nullable` column adds a UInt8 null-mask column (storage + processing overhead).

### LowCardinality

Use `LowCardinality(String)` for string columns with fewer than ~10,000 distinct values:

```sql
service         LowCardinality(String),    -- ~8 services
action          LowCardinality(String),    -- ~50 actions
category        LowCardinality(String),    -- ~6 categories
outcome         LowCardinality(String),    -- success/failure/denied
```

### Partitioning

```sql
PARTITION BY toYYYYMM(timestamp)
```

Use month-based partitioning for time-series data. Ensures efficient TTL expiry and partition pruning.

### TTL

```sql
TTL timestamp + INTERVAL 1 YEAR
```

Always define TTL on event tables. Configurable per tenant via application logic.

---

## 7. MariaDB-Specific Conventions

### UUID Type

Use the native `UUID` type (MariaDB 10.7+). It stores 16 bytes internally and displays as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`. Always generate with `uuid_v7()`.

### DATETIME Precision

Use `DATETIME(3)` (millisecond precision) for all timestamp columns to match the API's ISO-8601 `.SSS` format:

```sql
created_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3),
updated_at DATETIME(3) NOT NULL DEFAULT CURRENT_TIMESTAMP(3) ON UPDATE CURRENT_TIMESTAMP(3)
```

### Timezone

All connections **MUST** set `time_zone = '+00:00'` to ensure `CURRENT_TIMESTAMP`, `NOW()`, and default values produce UTC. `DATETIME` has no timezone semantics — without this setting, server-local time is used.

### Character Sets

Use `utf8mb4` character set and `utf8mb4_unicode_ci` collation for all string columns.

---

## 8. Boolean Fields

Use `BOOL` (MariaDB alias for `TINYINT(1)`) with `is_` prefix:

**MariaDB:**

```sql
is_enabled  BOOL NOT NULL DEFAULT TRUE,
is_deleted  BOOL NOT NULL DEFAULT FALSE
```

**ClickHouse:**

```sql
is_deleted  UInt8 DEFAULT 0
```

**Rules:**
- Always prefix with `is_` — `is_enabled`, `is_deleted`, `is_bot`, `is_active`
- Always `NOT NULL` with an explicit `DEFAULT` — a boolean should never be unknown
- ClickHouse: use `UInt8` (0/1), not `LowCardinality(String)` — booleans are not categorical strings
- Never use `ENUM('yes','no')` or string representations

---

## 9. Soft-Delete

Use `deleted_at` timestamp (not boolean flag) for soft-delete:

**MariaDB:**

```sql
deleted_at DATETIME(3) NULL DEFAULT NULL
```

**Rules:**
- `NULL` means active/not deleted; non-NULL means deleted at that timestamp
- Aligns with [API Guideline](../api-guideline/API.md) §3 which specifies `deleted_at` as optional standard field
- Application queries add `WHERE deleted_at IS NULL` for active records
- Prefer `deleted_at` over `is_deleted BOOL` — the timestamp provides audit value (when was it deleted?)
- For ClickHouse, use `is_deleted UInt8 DEFAULT 0` — ClickHouse does not benefit from nullable timestamps for filtering, and `ReplacingMergeTree` uses `is_deleted` for tombstone semantics

---

## 10. Observation Timestamps

For records that track when something was first/last observed (e.g., unmapped aliases, sync status):

| Column | Type (MariaDB) | Nullable | Description |
|--------|----------------|----------|-------------|
| `first_observed_at` | `DATETIME(3) NOT NULL` | No | When the record was first seen |
| `last_observed_at` | `DATETIME(3) NOT NULL` | No | When the record was most recently seen |

**Rules:**
- `first_observed_at` is immutable after insert (like `created_at`)
- `last_observed_at` updates on every observation
- These differ from `created_at`/`updated_at` — a record may be created once but observed many times without being "updated" (no attribute change)
- Follow the `_at` suffix convention — not `first_seen` / `last_seen`

---

## 11. String Length Tiers

Use consistent `VARCHAR` lengths based on content type:

| Tier | Length | Use for | Examples |
|------|--------|---------|----------|
| **Short** | `VARCHAR(50)` | Type codes, enum-like values, short identifiers | `value_type`, `assignment_type`, `source` |
| **Medium** | `VARCHAR(100)` | System names, rule names, attribute names | `insight_source_type`, `attribute_name`, `condition_type` |
| **Standard** | `VARCHAR(255)` | Human-readable names, display values | `display_name`, `name`, `role`, `location` |
| **Long** | `VARCHAR(500)` | Emails, URLs, external identifiers, user agents | `value`, `email`, `source_account_id`, `actor_user_agent` |
| **Unbounded** | `TEXT` | Paths, free-form text, reasons, descriptions | `path`, `reason`, `description` |

**Rules:**
- Pick the tier that fits the content, not the "maximum possible length"
- `TEXT` should only be used when length is genuinely unpredictable
- Do NOT use `VARCHAR(500)` for a field that will always be under 50 characters
- ClickHouse: always use `String` (no length limit) or `FixedString(N)` for fixed-width codes

---

## 12. Anti-Patterns

| Anti-pattern | Why | Do instead |
|-------------|-----|-----------|
| `INT AUTO_INCREMENT` PK + separate UUID column | Unnecessary complexity at Insight scale; two IDs to manage | `id UUID DEFAULT uuid_v7() PRIMARY KEY` |
| Bare `tenant_id` | Collides with source system field names (e.g., Azure `tenant_id`) | `insight_tenant_id UUID NOT NULL` |
| `tenant_id VARCHAR(100)` | Inconsistent with project UUID convention; larger storage | `insight_tenant_id UUID NOT NULL` |
| `source_system VARCHAR` / bare `source` | Ambiguous naming; no instance-level granularity | `insight_source_type` + `insight_source_id` pair |
| `performed_by VARCHAR(100)` (username string) | Breaks on rename; cannot join to persons table | `actor_person_id UUID` (FK to persons.id) |
| `valid_from` / `valid_to` or `owned_from` / `owned_until` | Multiple naming conventions for the same concept | `effective_from` / `effective_to` everywhere |
| `first_seen` / `last_seen` (no `_at` suffix) | Breaks timestamp naming convention | `first_observed_at` / `last_observed_at` |
| `is_deleted BOOL` in MariaDB | Loses "when deleted" information | `deleted_at DATETIME(3) NULL` |
| `TIMESTAMP` for MariaDB columns | Implicit timezone conversion; 2038 limit | `DATETIME(3)` |
| UUID first in ClickHouse `ORDER BY` | Destroys data skipping and compression | `insight_tenant_id` first, UUID last |
| `Nullable(String)` in ClickHouse | Storage overhead from null-mask column | Empty string `''` as default |
| `Enum8`/`Enum16` in ClickHouse | Hard to evolve (adding values requires ALTER) | `LowCardinality(String)` |

---

## 13. Proposals

Conventions proposed but not yet confirmed. Will be finalized when the corresponding domain modules are designed.

### P1: Hierarchy & Tree Fields

> **Scope**: org-chart module (future)

For materialized path hierarchies (org units, categories):

| Column | Type (MariaDB) | Description |
|--------|----------------|-------------|
| `parent_id` | `UUID NULL` | FK to same table's `id`; `NULL` = root node |
| `path` | `TEXT NOT NULL` | Materialised path (e.g., `/company/engineering/platform`) |
| `depth` | `INT NOT NULL DEFAULT 0` | Nesting level (root = 0) |

Rules: `path` uses `/`-delimited segments; `depth` derived from path; application prevents circular references; index `path` for prefix queries.

### P2: Status & Enum Fields

> **Scope**: per-domain DESIGN documents

Use MariaDB `ENUM` for fixed, small value sets. Name the column by what it represents (e.g., `status`, `role`, `outcome`). Values are `snake_case`, lowercase. ClickHouse equivalent: `LowCardinality(String)`.

Concrete enum values will be defined per domain.

### P3: Actor / Audit Attribution

> **Scope**: audit service, identity service

| Column | Type (MariaDB) | Description |
|--------|----------------|-------------|
| `actor_person_id` | `UUID NOT NULL` | FK to persons.id -- who performed the action |
| `actor_ip` | `VARCHAR(45)` | Client IP (IPv4 or IPv6) |
| `actor_user_agent` | `VARCHAR(500)` | Client User-Agent |

Rules: always UUID FK, never username strings. For grant/revoke: `granted_by UUID`, `revoked_by UUID`. For resolution: `resolved_by UUID`.

### P4: Confidence & Scoring Fields

> **Scope**: identity-resolution module

| Column | Type | Description |
|--------|------|-------------|
| `confidence` | `DECIMAL(3,2)` | Score 0.00-1.00 |
| `completeness_score` | `FLOAT` | Fraction of non-null attributes (0.0-1.0) |

### P5: JSON / Flexible Storage

> **Scope**: per-domain DESIGN documents

Use JSON columns sparingly for genuinely dynamic data (`config`, `parameters`, `snapshot_before`, `snapshot_after`). JSON field names follow `snake_case`. Do NOT store queryable data in JSON. ClickHouse: use `String` type.

### P6: Hash & Change Detection

> **Scope**: identity-resolution, connector sync

| Column | Type | Description |
|--------|------|-------------|
| `record_hash` | `VARCHAR(64)` | SHA-256 hex of canonical attribute set for change detection |
| `version` | `INT NOT NULL DEFAULT 1` | Monotonic counter for optimistic locking |

### P7: Business Temporal Validity in MariaDB

> **Scope**: org-chart module (future)

**Question**: How should org membership periods (`effective_from`/`effective_to`) be managed in MariaDB? Specifically: uniqueness enforcement (only one active membership per person+org_unit), query patterns for "who was in this unit on date X", and indexing strategy.

**Context**: The `effective_from`/`effective_to` naming convention is adopted (section 4.2), but the concrete schema design for temporal tables in MariaDB will depend on the org-chart module architecture.

---

## 14. Proposals with Known Contradictions

Patterns that were initially considered but contradict the adopted SCD mechanism (dbt-macros with `*_snapshot` and `fields_history` tables in ClickHouse). Documented for context — not to be implemented.

### SCD Type 2 in MariaDB

An earlier draft proposed SCD2 versioning directly in MariaDB tables (entity anchor table + versioned rows with `version INT`, composite PK `(entity_id, version)`). This contradicts the adopted approach where SCD2/SCD3 is implemented via **dbt-macros** that populate `*_snapshot` and `fields_history` tables in ClickHouse. MariaDB stores only the **current state** of each entity; historical versions are managed by the analytics pipeline.

### MariaDB Partial Index Workaround

An earlier draft proposed a generated column `is_current` to emulate PostgreSQL partial unique indexes in MariaDB (`WHERE effective_to IS NULL`). This workaround is unnecessary if SCD2 versioning is not in MariaDB. For **business temporal validity** tables (e.g., org memberships with `effective_from`/`effective_to`), the uniqueness constraint will be defined when the org-chart module is designed.

---

## 15. Known Convention Violations

Existing code and specs that conflict with the adopted conventions. To be updated as part of follow-up work.

### Code

| File | Violation | Convention |
|------|-----------|------------|
| `src/ingestion/connectors/hr-directory/bamboohr/dbt/to_class_people.sql:16` | `CAST(NULL AS Nullable(DateTime))` for `valid_to` | ClickHouse Nullable avoidance (section 6); use sentinel value (e.g., `'1970-01-01'`) instead of `Nullable` |
| `src/ingestion/connectors/hr-directory/bamboohr/dbt/to_class_people.sql:12` | bare `tenant_id` | `insight_tenant_id` (section 3.1) |
| `src/ingestion/connectors/hr-directory/bamboohr/dbt/to_class_people.sql:15` | `valid_from` / `valid_to` | `effective_from` / `effective_to` (section 4.2) |
| `src/ingestion/connectors/ai/claude-admin/dbt/claude_admin__ai_api_usage.sql` | `insight_source_id` without `insight_source_type` | Source tracking fields must co-occur (section 3.2) |
| `src/ingestion/connectors/ai/claude-admin/dbt/claude_admin__ai_api_usage.sql` | bare `tenant_id` | `insight_tenant_id` (section 3.1) |

### Spec documents

See PR #55 summary for the full list of spec-level violations across ~25 documents (~270+ bare `tenant_id` references, `valid_from/valid_to`, `performed_by VARCHAR`, `INT` PKs, `TIMESTAMP` types).
