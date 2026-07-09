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
- [Update (2026-06-03): silver moved to delete+insert](#update-2026-06-03-silver-moved-to-deleteinsert)
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
- Bad: consumers must remember to use `FINAL` or `argMax` (interim state between merges may show duplicates) — mitigated by documenting the contract in this ADR and `.cf-studio/config/rules/architecture.md`
- Bad: no automatic enforcement of "engine = RMT, order_by = unique_key" — mitigated by Constructor Studio skill `/check-dbt-conventions` (LLM-based) and code review

### Confirmation

- `cfs validate` confirms code markers reference this ADR/DESIGN ID (audit trail)
- Constructor Studio skill `/check-dbt-conventions` reads every `.sql` model and asserts engine + order_by are correct (correctness check, LLM-based)
- Visual / grep audit: `grep -r "engine=" src/ingestion/silver/ | grep -v ReplacingMergeTree` should return only commented-out exceptions

## Update (2026-06-03): silver moved to `delete+insert`

**Trigger.** A misconfigured Airbyte sync ran `full_refresh | append` (instead of
`incremental`) across all connectors of a deployment for a period, re-appending
every source row. Bronze accumulated many duplicates per key. RMT only collapses on
background merge, so the duplicates were still live at query time and propagated
through staging into silver. Because the read-time-`FINAL` discipline of option A
is unenforced, a consumer that forgot `FINAL` — the gold view
`insight.collab_bullet_rows` — double-counted rows and surfaced impossible
metrics (e.g. Collaboration "Active days" = 42 days/month).

**Decision change.** For **silver** models (`class_*`, `fct_*`, `identity_inputs`)
that are `materialized='incremental'`, switch from `incremental_strategy='append'`
to **`incremental_strategy='delete+insert'` keyed on `unique_key`**. The silver
table is then physically at most one row per `unique_key`, so **any consumer
(gold views, the product, ad-hoc queries) can read silver without `FINAL`** and
never see duplicates. This is the property option A could not guarantee.

**Scope and rationale.**
- This is option **B applied to silver only**. The original "double-cost" argument
  against B is moot here: silver write volumes are small (daily aggregates), and
  correctness for un-disciplined consumers outweighs the extra delete cost.
- **Staging stays RMT + `append`** (cheap). Silver reads staging through
  `union_by_tag`, which now deduplicates the union to one row per `unique_key`
  (`QUALIFY ROW_NUMBER() … ORDER BY _version DESC`, or `LIMIT 1 BY` for versionless
  sources) — so transient staging duplicates never reach silver.
- **Bronze stays RMT.** Models that **aggregate** over bronze (`count`/`sum`)
  must still dedup the bronze read explicitly (`FINAL` for RMT-promoted bronze,
  `LIMIT 1 BY unique_key` for non-promoted MergeTree bronze), because aggregation
  bakes inflation into a single row that no downstream dedup can undo
  (e.g. `bitbucket_cloud__commits`, `claude_admin__ai_dev_usage`).
- The `snapshot()` macro now reads its bronze source with `FINAL`, so transient
  duplicates do not create spurious SCD2 history versions.
- `materialized='table'` silver models (`class_people`, `mtr_git_person_*`,
  `class_hr_working_hours`) are rebuilt in full each run and already collapse to
  one row per key via the `union_by_tag` dedup — left unchanged.

**Enforcement.** `src/ingestion/dbt/audit_rmt_read_dedup.py` checks every RMT read
across staging, silver, and the gold-view migrations and fails (exit≠0) on an
un-deduped read — usable as a CI gate. One-time cleanup of already-accumulated
duplicates: `dbt run --full-refresh --select tag:silver`.

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

This ADR was implemented in the same session as it was authored — every silver model and every relevant staging model now follows pattern A. See `.cf-studio/config/rules/architecture.md` §"dbt Materialization Conventions" for the operational summary.

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
