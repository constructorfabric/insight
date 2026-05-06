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
2. **Engine = MergeTree** with a sensible `ORDER BY` so the table is a
   valid drop-in.
3. **Idempotent** — guarded with `ch_table_exists` so re-runs of
   `init.sh` are safe.
4. **Replaced on first real run.** Airbyte (for bronze) and dbt (for
   silver) drop the placeholder and recreate with the real schema.
   ClickHouse rebinds VIEW resolution on each SELECT, so the table
   replacement is invisible to consumers.

When adding a NEW migration that references a NEW silver / bronze
table, the migration author MUST also extend `create-bronze-placeholders.sh`
with a placeholder for that table.

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
  placeholder. Detection is live-test: there is no static check.
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
