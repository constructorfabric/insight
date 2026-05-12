---
status: accepted
date: 2026-04-30
decision-makers: roman.mitasov
---

# Promote bronze MergeTree → ReplacingMergeTree on first dbt run


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [A. dbt-managed atomic migration via EXCHANGE TABLES](#a-dbt-managed-atomic-migration-via-exchange-tables)
  - [B. Live with MergeTree bronze; dedup-on-read in staging](#b-live-with-mergetree-bronze-dedup-on-read-in-staging)
  - [C. Modify Airbyte CH destination to write RMT directly](#c-modify-airbyte-ch-destination-to-write-rmt-directly)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-dataflow-adr-promote-bronze-to-rmt`
## Context and Problem Statement

Airbyte creates ClickHouse destination tables as plain `MergeTree` and writes them with `destinationSyncMode='append'` (mandated by `cpt-dataflow-constraint-airbyte-append` to avoid OOM on large streams). Full-refresh streams therefore accumulate N copies per logical entity across N syncs. Plain `MergeTree` cannot dedup; downstream staging models must either pay the dedup cost on every read (`QUALIFY ROW_NUMBER` over `_airbyte_extracted_at`) or accept duplicates. We needed a way to make bronze tables themselves dedup-aware.

## Decision Drivers

- Airbyte `append` is non-negotiable (OOM if `append_dedup`, data loss on retry if `overwrite`)
- Bronze duplicates must collapse without paying full-refresh dedup cost on every read
- Migration must be safe, idempotent, and not require manual SQL ops per tenant
- ClickHouse does NOT support `ALTER TABLE ... MODIFY ENGINE` — must use CREATE + EXCHANGE + DROP

## Considered Options

- **A.** dbt-managed atomic migration via `CREATE TABLE __swap AS SELECT * FROM <tbl>` + `EXCHANGE TABLES` + `DROP __swap`, packaged in a reusable macro `promote_bronze_to_rmt(table, order_by)` and invoked from per-connector bootstrap models
- **B.** Live with `MergeTree` bronze; do dedup-on-read in every staging model via `QUALIFY ROW_NUMBER OVER (PARTITION BY unique_key ORDER BY _airbyte_extracted_at DESC) = 1`
- **C.** Modify Airbyte's ClickHouse destination connector to accept an engine declaration and write RMT directly

## Decision Outcome

Chosen option: **"A. dbt-managed atomic migration"**, because it lets us keep Airbyte unmodified, gives bronze the right engine semantics (dedup by `_airbyte_extracted_at` on merge / `FINAL`), and is fully idempotent so it slots cleanly into the standard `dbt run` lifecycle without per-tenant manual ops.

### Consequences

- Good: bronze becomes self-deduplicating after first dbt run — staging models can `SELECT * FROM bronze` and (with `FINAL`) get one row per `unique_key`
- Good: idempotent (subsequent runs check `system.tables.engine` and skip if already RMT)
- Good: atomic via `EXCHANGE TABLES` (CH 22.7+) — no window of inconsistency for concurrent readers after the migration commits
- Good: per-connector bootstrap model carries the table list co-located with the connector (e.g. `jira__bronze_promoted.sql` lives next to other Jira staging models)
- Bad: requires CH 22.7+ for `EXCHANGE TABLES` (we're on 25+, so fine)
- Bad: doubles disk usage temporarily during CTAS (mitigated by running first promotion when source is small)
- Bad: race window between `CREATE __swap AS SELECT *` and `EXCHANGE` — rows written to the source table during this window are lost (Airbyte INSERTs not yet flushed to the swap copy)
  - Mitigation: schedule the FIRST promotion of each table when no Airbyte sync is in flight; subsequent runs are no-ops
- Bad: replicated tables (`Replicated*MergeTree`) are NOT supported by the macro (would need ZK path); current single-node deployment is fine

### Confirmation

- `cpt validate` confirms the bootstrap model and `promote_bronze_to_rmt` macro carry traceability markers referencing this ADR
- Manual check: `clickhouse-client -q "SELECT name, engine FROM system.tables WHERE database LIKE 'bronze_%'"` should show `Replacing*` for every promoted table after first dbt run
- The macro itself logs `info=True` on every action so the dbt run output records the migration

## Pros and Cons of the Options

### A. dbt-managed atomic migration via EXCHANGE TABLES

- Good: idempotent, automated, per-connector configurable
- Good: requires no Airbyte changes
- Good: CTAS preserves schema (column types, defaults), so subsequent Airbyte schema migrations (`ALTER TABLE ADD COLUMN`) compose
- Bad: a small race window during the swap (acceptable in practice)
- Bad: temp disk doubling during CTAS

### B. Live with MergeTree bronze; dedup-on-read in staging

- Good: zero migration risk
- Good: no temporary disk usage
- Bad: every staging model pays `QUALIFY ROW_NUMBER` cost on every run (CPU + memory)
- Bad: leaks dedup logic into every connector's staging model (boilerplate creep)
- Bad: doesn't help direct-bronze readers (Rust enrich, ad-hoc analysts) — they still see duplicates

### C. Modify Airbyte CH destination to write RMT directly

- Good: cleanest long-term — no migration needed at all
- Good: bronze is RMT from day one
- Bad: requires upstream Airbyte change (PR + review + release cycle)
- Bad: blocks rollout until Airbyte ships our change
- Bad: doesn't solve the existing-deployment migration problem

## More Information

Implementation: macro `promote_bronze_to_rmt(table, order_by)` in `src/ingestion/dbt/macros/promote_bronze_to_rmt.sql`. Per-connector wrapper as a dbt model with `pre_hook` calls (one per bronze table), e.g., `src/ingestion/connectors/task-tracking/jira/dbt/jira__bronze_promoted.sql`. Other connector models declare `-- depends_on: {{ ref('<connector>__bronze_promoted') }}` so dbt's DAG fires the migration before any model reads bronze.

The migration uses `_airbyte_extracted_at` as the `_version` column for RMT — Airbyte populates this on every row, monotonic across syncs, deterministic.

The Airbyte append-only constraint that motivates this whole ADR is enforced in `src/ingestion/airbyte-toolkit/connect.sh`:

```python
# Bronze is always plain append; dedup happens in silver via unique_key.
# Destination-side dedup (append_dedup) buffers all records in memory
# until stream COMPLETE — OOMs on large streams and loses all data
# on mid-stream pod death. Overwrite has the same problem on retries.
dest_sync_mode = "append"
```

## Traceability

- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **Sibling ADRs**:
  - `cpt-dataflow-adr-rmt-with-version-and-unique-key` (parent — establishes RMT as the universal engine choice)

This decision directly addresses the following design elements:

* `cpt-dataflow-principle-promote-bronze` — implements the migration mechanism
* `cpt-dataflow-constraint-airbyte-append` — provides the constraint that necessitates this migration
* `cpt-dataflow-seq-bronze-promotion` — describes the runtime sequence
* `cpt-dataflow-component-bronze` — promotes the bronze layer to RMT after first dbt run
