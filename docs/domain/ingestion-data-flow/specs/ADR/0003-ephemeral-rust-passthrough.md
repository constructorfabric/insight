---
status: accepted
date: 2026-04-30
decision-makers: roman.mitasov
---

# Ephemeral materialization for Rust-owned staging pass-through


<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [A. ephemeral materialization](#a-ephemeral-materialization)
  - [B. view materialization](#b-view-materialization)
  - [C. Direct source ref in silver, drop the dbt-side wrapper](#c-direct-source-ref-in-silver-drop-the-dbt-side-wrapper)
  - [D. Tag the source declaration directly, extend `union_by_tag`](#d-tag-the-source-declaration-directly-extend-unionbytag)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-dataflow-adr-ephemeral-rust-passthrough`
## Context and Problem Statement

The Jira `class_task_field_history` silver model is populated from `staging.jira__task_field_history`, which is written **by the Rust `jira-enrich` binary**, not by dbt. The dbt-side needs a node in the DAG that:

1. Carries the tag `silver:class_task_field_history` so `union_by_tag` finds it
2. Provides a `ref()`-able placeholder so silver's depends_on chain stays explicit
3. Does NOT create a redundant DB object (the staging table already exists)

Initially this was a `view` over the source — a 1-line passthrough materialized as `staging.jira__task_field_history_tagged`. After we moved `unique_key` computation into the Rust binary itself, the view became pure `SELECT *` — a useless physical object.

## Decision Drivers

- The `union_by_tag` macro discovers tagged dbt models, not sources — so the dbt-side node must exist as a model
- A pure `SELECT * FROM source(...)` view adds zero-value overhead (a DB object that aliases another DB object)
- The pattern should be reusable when other Rust-owned (or externally-written) staging tables appear

## Considered Options

- **A.** `materialized='ephemeral'` — dbt creates no DB object; the SELECT is inlined as a CTE in any downstream model that `ref()`s it. Tags still attach. `union_by_tag` macro patched to handle ephemeral nodes (skip the relation existence check)
- **B.** `materialized='view'` (current state before this ADR) — creates `staging.<model>_tagged` view aliasing the source
- **C.** Drop the dbt-side wrapper entirely; in silver, directly reference `source('staging_jira', 'jira__task_field_history')`
- **D.** Tag the source declaration in `schema.yml` and extend `union_by_tag` to also iterate `graph.sources`

## Decision Outcome

Chosen option: **"A. ephemeral materialization"**, because it removes the redundant DB object while preserving the multi-source pattern (silver still uses `union_by_tag('silver:class_task_field_history')` and would automatically pick up a future second connector for task field history, e.g. Linear).

### Consequences

- Good: zero database objects per Rust-owned table on the dbt side
- Good: the `union_by_tag` pattern stays uniform — no per-class special case in silver models
- Good: future Rust-owned staging tables follow the same pattern (just add an ephemeral wrapper with the tag)
- Bad: `union_by_tag` macro now has a small ephemeral-aware branch — slight increase in macro complexity
- Bad: ephemeral cannot have hooks (per dbt docs), so the migration / DDL must live elsewhere (here: `on-run-start` macro `create_task_field_history_staging`)

### Confirmation

- `dbt parse` succeeds — confirms the macro patch is syntactically correct and the ephemeral model is in the DAG
- `dbt list --select tag:jira` shows `jira__task_field_history` in the list (DAG node exists)
- `clickhouse-client -q "SHOW TABLES FROM staging LIKE '%tagged%'"` returns nothing (no view object created)
- `dbt compile --select class_task_field_history` produces SQL with the source reference inlined as a subquery

## Pros and Cons of the Options

### A. ephemeral materialization

- Good: zero DB object footprint
- Good: SQL is inlined; ClickHouse optimizer collapses the trivial `SELECT *` wrapper at planning time
- Good: tag-based discovery via `union_by_tag` continues to work after a small macro patch
- Good: easily reusable for any other externally-written staging table
- Bad: ephemeral nodes cannot have `pre_hook` / `post_hook` / tests (dbt limitation) — irrelevant here, but worth noting
- Bad: `adapter.get_relation` returns `None` for ephemeral, so `union_by_tag` had to be taught to handle this branch

### B. view materialization

- Good: simple, no macro changes needed
- Good: works out of the box with `union_by_tag`
- Bad: creates a DB object that adds zero value (just aliases the source)
- Bad: confusing to operators — "what's `_tagged` for? why is there a view next to the table?"

### C. Direct source ref in silver, drop the dbt-side wrapper

- Good: simplest possible
- Bad: hardcodes `source('staging_jira', 'jira__task_field_history')` in silver — when a second connector for task field history arrives (Linear, YouTrack), silver has to be edited (lose the auto-discovery)
- Bad: inconsistent with all other `class_task_*` models which use `union_by_tag`

### D. Tag the source declaration directly, extend `union_by_tag`

- Good: even cleaner than ephemeral — no model file at all, just a tagged source
- Bad: bigger macro change (must iterate both `graph.nodes` and `graph.sources`, handle source resource_type)
- Bad: source-level tags are less ergonomic to add per-table than a model-level config block
- Bad: blast radius across all `union_by_tag` callers — riskier change for the same payoff as ephemeral

## More Information

Implementation:

```sql
-- src/ingestion/connectors/task-tracking/jira/dbt/jira__task_field_history.sql
{{ config(
    materialized='ephemeral',
    tags=['jira', 'silver:class_task_field_history']
) }}
SELECT * FROM {{ source('staging_jira', 'jira__task_field_history') }}
```

The `union_by_tag` macro patch:

```jinja
{%- if node.config.materialized == 'ephemeral' -%}
  {#- Ephemeral models have no DB relation; dbt inlines them as CTE on ref(). -#}
  {%- do models.append(node) -%}
{%- else -%}
  {%- set rel = adapter.get_relation(...) -%}
  {%- if rel -%} ... {%- endif -%}
{%- endif -%}
```

This pattern is currently used only for `staging.jira__task_field_history`. Future candidates: any external-process-written staging table where dbt only needs to attach a tag.

## Traceability

- **DESIGN**: [DESIGN.md](../DESIGN.md)
- **Sibling ADRs**:
  - `cpt-dataflow-adr-rmt-with-version-and-unique-key` (parent — establishes the silver shape ephemeral feeds into)
  - `cpt-dataflow-adr-unique-key-formula` (defines what the Rust binary must compute and write)

This decision directly addresses the following design elements:

* `cpt-dataflow-principle-ephemeral-passthrough` — formalizes the ephemeral choice as the project-wide pattern for externally-written staging tables
* `cpt-dataflow-component-rust-enrich` — defines how the Rust output reaches silver
* `cpt-dataflow-seq-rust-staging` — the runtime flow that uses the ephemeral wrapper
