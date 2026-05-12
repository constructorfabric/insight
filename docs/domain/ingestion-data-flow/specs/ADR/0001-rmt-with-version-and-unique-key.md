---
status: accepted
date: 2026-04-30
decision-makers: roman.mitasov
---

# ReplacingMergeTree(_version) + ORDER BY (unique_key) for every dbt-managed table


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [A. RMT(_version) + ORDER BY (unique_key) + read-time FINAL/argMax](#a-rmtversion--order-by-uniquekey--read-time-finalargmax)
  - [B. delete+insert + RMT (or plain MT)](#b-deleteinsert--rmt-or-plain-mt)
  - [C. Composite ORDER BY (per-table natural keys)](#c-composite-order-by-per-table-natural-keys)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-dataflow-adr-rmt-with-version-and-unique-key`
## Context and Problem Statement

dbt models in `staging` and `silver` initially had inconsistent dedup strategies — some used `incremental_strategy='delete+insert'`, some had no engine declaration (default `MergeTree`), some had composite ORDER BY tuples. Several silver models accumulated duplicates because background merges of `ReplacingMergeTree` had nothing to merge on (no `_version`) or because the model was a view that re-executed UNION ALL on every read. We needed a single uniform contract that every dbt-managed table follows so dedup works deterministically without per-table reasoning.

## Decision Drivers

- Consumers should not need per-table knowledge of dedup keys — one column (`unique_key`) for all tables
- Dedup must be deterministic (predictable winner when duplicates collide)
- Read pattern must be uniform: `FINAL` or `argMax(... ORDER BY _version)`
- No double-cost dedup (delete+insert + RMT was paying twice)
- Cross-connector silver UNION ALL must not collide between connectors

## Considered Options

- **A.** `ReplacingMergeTree(_version)` + `ORDER BY (unique_key)` + read-time `FINAL`/`argMax` (one uniform pattern; versionless RMT only for full-refresh tables whose staging upstream lacks `_version`)
- **B.** `incremental_strategy='delete+insert'` (with `unique_key` config) + RMT or plain MergeTree
- **C.** Composite ORDER BY per table (e.g., `(insight_source_id, data_source, comment_id)`) so each table's natural key is the dedup key

## Decision Outcome

Chosen option: **"A. RMT(_version) + ORDER BY (unique_key) + read-time FINAL/argMax"**, because it gives a single uniform contract every model follows, dedup is deterministic by `_version`, write cost is minimal (just INSERT), and cross-connector UNION ALL is collision-safe because every connector's `unique_key` includes `tenant-source` prefix.

For genuinely full-refresh tables — only three cases per `cpt-dataflow-principle-incremental-default`:

1. Full-refresh source with current-state semantics (`class_people`, `class_hr_working_hours`)
2. Aggregations that must scan all data (`mtr_git_person_totals`, `mtr_git_person_weekly`)
3. Explode/fan-out staging models with full-refresh bronze (`jira__changelog_items`)

For these three categories, use **versionless RMT** (`engine='ReplacingMergeTree'` without the `_version` argument). The table is rebuilt from scratch each run; within-run UNION ALL collisions are the only dedup case and "any one wins" is acceptable.

For all other models (event/append semantics): use `RMT(_version)` + `incremental` + `WHERE _version > max(_version)` filter. If upstream staging lacks `_version`, the staging model itself MUST be amended to project one (typically `toUnixTimestamp64Milli(_airbyte_extracted_at)`).

### Consequences

- Good: every silver model has the same shape — easier to read, write, and review
- Good: write path stays cheap (append + RMT merge in background)
- Good: cross-connector silver UNION ALL is collision-safe by construction (each connector's `unique_key` is globally unique within tenant)
- Good: ORDER BY a single column gives compact primary index
- Bad: consumers must remember to use `FINAL` or `argMax` (interim state between merges may show duplicates) — mitigated by documenting the contract in this ADR and `cypilot/config/rules/architecture.md`
- Bad: no automatic enforcement of "engine = RMT, order_by = unique_key" — mitigated by Cypilot skill `/check-dbt-conventions` (LLM-based) and code review

### Confirmation

- `cpt validate` confirms code markers reference this ADR/DESIGN ID (audit trail)
- Cypilot skill `/check-dbt-conventions` reads every `.sql` model and asserts engine + order_by are correct (correctness check, LLM-based)
- Visual / grep audit: `grep -r "engine=" src/ingestion/silver/ | grep -v ReplacingMergeTree` should return only commented-out exceptions

## Pros and Cons of the Options

### A. RMT(_version) + ORDER BY (unique_key) + read-time FINAL/argMax

- Good: write cost = INSERT (cheap)
- Good: dedup is canonical CH idiom
- Good: one read pattern fits all (`FINAL` / `argMax`)
- Good: composes well with `union_by_tag` UNION ALL (no per-source decision needed for dedup)
- Bad: requires read-time discipline (or wrapper views with `FINAL`)
- Bad: interim state has duplicates until merge / FINAL

### B. delete+insert + RMT (or plain MT)

- Good: target table is always clean (no read-time discipline needed)
- Good: works with plain MergeTree (no engine choice required)
- Bad: `LIGHTWEIGHT DELETE` is more expensive than INSERT
- Bad: stacks two dedup mechanisms when used with RMT (delete+insert AND merge)
- Bad: requires per-model `unique_key` config that must match the table's natural key
- Bad: write path scales worse on large incremental batches

### C. Composite ORDER BY (per-table natural keys)

- Good: ORDER BY captures the natural key directly (semantically transparent)
- Bad: every table has a different ORDER BY — no uniform shape
- Bad: breaks `union_by_tag` UNION ALL safety when combining sources whose composite keys overlap (e.g., two task connectors with same `comment_id`)
- Bad: no project-wide convention to validate against
- Bad: forces consumers to know the right ORDER BY columns when writing dedup queries

## More Information

The decision was reached after auditing all 34 silver and 60 connector staging models. Pre-decision state had: 19 silver `class_*` models on RMT(_version) without `unique_key` projection (composite ORDER BY); 5 silver views with no dedup at all; 2 silver tables with no engine declaration; staging Jira models all using composite ORDER BY despite bronze having `unique_key`.

This ADR was implemented in the same session as it was authored — every silver model and every relevant staging model now follows pattern A. See `cypilot/config/rules/architecture.md` §"dbt Materialization Conventions" for the operational summary.

## Traceability

- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **Sibling ADRs**:
  - `cpt-dataflow-adr-promote-bronze-to-rmt` (depends on RMT decision)
  - `cpt-dataflow-adr-ephemeral-rust-passthrough` (extends to Rust-owned tables)
  - `cpt-dataflow-adr-unique-key-formula` (defines what `unique_key` contains)

This decision directly addresses the following design elements:

* `cpt-dataflow-principle-rmt-with-version` — engine + order_by mandate
* `cpt-dataflow-principle-incremental-default` — incremental as default, table only for three justified cases
* `cpt-dataflow-principle-staging-then-union` — uniform shape enables `union_by_tag` to compose UNION ALL safely
* `cpt-dataflow-component-staging` — every staging model materializes RMT(_version) ORDER BY (unique_key)
* `cpt-dataflow-component-silver` — every silver model materializes RMT(_version) ORDER BY (unique_key)
