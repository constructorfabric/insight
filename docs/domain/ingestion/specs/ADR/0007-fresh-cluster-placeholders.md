---
id: cpt-ingestion-adr-fresh-cluster-placeholders
status: accepted
date: 2026-05-05
---

# ADR-0007 — Fresh-cluster placeholders for silver / bronze tables

## Context

ClickHouse-side gold-view migrations in
`src/ingestion/scripts/migrations/*.sql` create `insight.*` views that
SELECT from:

- `silver.<dbt_model>` — built by `dbt run`
- `bronze_<source>.<stream>` — populated by Airbyte connector syncs

ClickHouse 25.3 validates referenced tables at `CREATE VIEW` time
(`UNKNOWN_TABLE` / `UNKNOWN_DATABASE` is fatal at parse). On a freshly
provisioned cluster, neither dbt nor the Airbyte connectors have run
yet, so every silver-dependent migration fails immediately — before
init.sh can even register the dbt-run Argo workflow that would build
the missing tables. This is a circular dependency:

```text
init.sh
  └─ run migrations            ← needs silver
  └─ register dbt-run workflow
       └─ Argo workflow runs `dbt run` later
            └─ builds silver
```

The pre-existing workaround — `scripts/create-bronze-placeholders.sh`,
which created empty bronze tables for connectors that hadn't synced
yet plus one silver table (`silver.class_comms_events`) — covered the
problem partially but lagged behind every new gold-view migration.
Each new migration that referenced an additional silver/bronze table
would re-introduce the failure on the next fresh install until
someone extended the placeholder list.

## Decision

**Every silver dbt-model and every bronze stream referenced by ANY
migration in `src/ingestion/scripts/migrations/` MUST have a
minimum-viable placeholder created by
`scripts/create-bronze-placeholders.sh` before migrations run.**

Rules for the placeholder:

1. **Schema is minimum-viable** — enough columns and approximate types
   for the referencing migrations to type-check the SELECT
   expressions. Full schema parity with the eventual dbt model /
   Airbyte stream is NOT required.
2. **Engine = ReplacingMergeTree** with the same version column the
   real owner uses (`_airbyte_extracted_at` for bronze placeholders,
   `_version` for silver placeholders) and a sensible `ORDER BY` so
   the table is a valid drop-in. Matching the real engine keeps reads
   against the placeholder semantically equivalent to reads against
   the eventually-built table.
3. **Bronze placeholders MUST include all four Airbyte CDK v2
   internal columns** in the exact order/types the destination
   emits:

   ```sql
   _airbyte_raw_id        String        DEFAULT toString(generateUUIDv4()),
   _airbyte_extracted_at  DateTime64(3) DEFAULT now64(3),
   _airbyte_meta          String        DEFAULT '{}',
   _airbyte_generation_id UInt32        DEFAULT 0
   ```

   The four types must match what the Airbyte CH destination v2 emits
   when it creates the table itself, and that contract has shifted
   between minor versions:

   - **`destination-clickhouse:2.0.x`** (our currently-pinned image,
     released 2025-07) emits `_airbyte_generation_id` as `UInt32`.
     Verified by inspecting tables Airbyte creates from scratch (e.g.
     `bronze_cursor.cursor_daily_usage` after dropping any placeholder,
     letting Airbyte create from scratch).
   - **`destination-clickhouse:2.1.0+`** (released 2025-09 via PR
     #65144 "Migrate to dataflow model") refactored schema generation
     through `ClickhouseTableSchemaMapper`, which maps the CDK
     `IntegerType` to `Int64`. So 2.1+ emits `_airbyte_generation_id`
     as `Int64`.

   The placeholder MUST track the destination's emit type. When
   bumping the Airbyte chart past 2.1.0, this placeholder column type
   must change to `Int64` in the same PR — otherwise
   `ensureSchemaMatches` will reject the placeholder on first sync.

   The Airbyte ClickHouse destination v2 calls
   `ensureSchemaMatches` at the start of every sync and refuses to
   write to a table whose schema is missing those four columns
   ("target table already exists in the destination, but does not
   contain Airbyte's internal columns"). Including them up-front
   makes the placeholder a valid Airbyte-managed table from day
   zero — Airbyte just writes to it. Silver placeholders do NOT
   need these columns; silver replacement is handled separately
   via the `on-run-start` drop hook described in rule 5.
4. **Idempotent** — guarded with `ch_table_exists` so re-runs of
   `init.sh` are safe.
5. **Replaced on first real run.**
   - **Bronze placeholders** are written to in place by Airbyte's
     destination on the first sync. Because rule 3 ships the four
     Airbyte CDK v2 internal columns up-front, the destination's
     `ensureSchemaMatches` check accepts the placeholder as a valid
     Airbyte-managed table and proceeds with normal schema evolution
     (adding stream columns as needed) instead of refusing to write.
   - **Silver placeholders** are *not* automatically replaced by dbt's
     incremental materialization (which `INSERT INTO`s an existing
     relation). To force replacement on the first real dbt run when
     staging has been materialised, every silver placeholder is
     created with the table comment `INSIGHT_PLACEHOLDER_v1`, and the
     project-level `on-run-start` hook
     `drop_silver_placeholders_at_start` (in
     `src/ingestion/dbt/macros/drop_silver_placeholders_at_start.sql`)
     drops any silver target that matches a **three-factor**
     signature: marker + zero rows + at least one staging model with
     tag `silver:<identifier>` materialised. The third factor is
     required because `union_by_tag`'s no-source-tables fallback
     emits `SELECT * FROM {{ this }} WHERE 1 = 0` to preserve schema
     — dropping the target before staging exists would break that
     fallback (`Code: 60 UNKNOWN_TABLE`). When the third factor
     fails, the placeholder is preserved and the silver materialise
     becomes a no-op (incremental INSERT of zero rows). The hook
     runs in `on-run-start` (not a per-model `pre-hook`) because
     dbt-clickhouse captures the target's existence at the start of
     materialization for `is_incremental()` — dropping inside a
     pre_hook leaves `is_incremental` as `True` and the compiled
     SQL still references the now-dropped target, producing a
     `SYNTAX_ERROR`. dbt's incremental materialization then sees no
     existing relation and creates the table with the model's full
     schema, engine, and ORDER BY.
   - ClickHouse rebinds VIEW resolution on each SELECT, so the
     replacement is invisible to gold-view consumers.

When adding a NEW migration that references a NEW silver / bronze
table, the migration author MUST also extend `create-bronze-placeholders.sh`
with a placeholder for that table. Silver placeholders MUST end with
`COMMENT 'INSIGHT_PLACEHOLDER_v1'` so the drop hook can pick them up.

## Consequences

### Positive
- Fresh cluster bring-up works end-to-end without manual intervention.
- Migrations remain idempotent and re-runnable on every `init.sh`
  invocation.
- The placeholder convention is a single, discoverable source of
  truth: one file, one pattern, one PR-checklist entry per new
  silver/bronze dependency.

### Negative — known tech debt
- The placeholder schema **drifts** from the dbt-model / Airbyte
  schema between PRs. If a placeholder is missing a column the new
  migration references, the install fails until someone extends the
  placeholder. Detection is live-test: there is no static check. The
  silver `drop_silver_placeholders_at_start` on-run-start hook mitigates one branch
  of this — schema mismatch between placeholder and dbt staging output
  no longer corrupts silver writes, because the placeholder is dropped
  before the first real INSERT — but it does not detect a placeholder
  missing a column the migration's gold-view SELECT references at
  init.sh time.
- The placeholder list **grows monotonically**. As migrations
  accumulate, so does the list. There is no automatic cleanup when a
  migration is removed.

### Future work
This ADR codifies the existing workaround as policy; it does not
solve the underlying ordering problem. Two architecturally cleaner
follow-ups are possible:

- **Split init.sh into `pre-dbt` and `post-dbt` phases.** Migrations
  that reference only bronze run pre-dbt; migrations that reference
  silver run post-dbt as a separate Argo step in the dbt-run workflow.
  Removes the need for silver placeholders entirely.
- **Move silver-dependent VIEW creation into dbt as `gold/` models.**
  dbt resolves dependencies via `ref()`; missing-bronze cases skip
  the model with a clean warning. Removes the need for ClickHouse
  migrations of this shape entirely.

Either follow-up is significantly larger than the placeholder
convention and was deliberately deferred so the bring-up path could
be unblocked without committing to an architectural redesign.

When the first follow-up lands, the silver placeholder blocks in
`scripts/create-bronze-placeholders.sh` AND the
`drop_silver_placeholders_at_start` macro + project-level on-run-start hook MUST be
removed in the same PR — the comment markers and the dbt-side drop
become dead code once silver tables are no longer pre-stubbed.

## References

- `src/ingestion/scripts/create-bronze-placeholders.sh` — the
  authoritative placeholder list.
- `src/ingestion/scripts/migrations/20260422100000_ic-kpis-honest-nulls.sql`
  — first migration that hit this problem on a fresh cluster after
  the move from `materialized='view'` to `materialized='table'` for
  silver models (PR #251).
- ADR-0002 (canonical alias-binding) — defines the column-rename
  convention referenced by some migrations.
- ADR-0006 (service-owned migrations) — sibling document on the
  MariaDB side, where the analogous problem is solved by per-service
  Migrators.
