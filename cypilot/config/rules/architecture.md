---
cypilot: true
type: project-rule
topic: architecture
generated-by: auto-config
version: 1.0
---

# Architecture

Data pipeline architecture and source categorisation rules for Constructor Insight. Apply when modifying pipeline architecture, adding new sources, or refactoring Bronze/Silver/Gold layers.

<!-- toc -->

- [Bronze / Silver / Gold Layers](#bronze--silver--gold-layers)
- [Identity Resolution](#identity-resolution)
- [Source Categories](#source-categories)
- [Collection Run Tables](#collection-run-tables)
- [dbt Pipeline Conventions (summary)](#dbt-pipeline-conventions-summary)
  - [Anti-patterns](#anti-patterns)
  - [Reference: shared macros](#reference-shared-macros)
  - [Validation](#validation)
- [Critical Files](#critical-files)

<!-- /toc -->

## Bronze / Silver / Gold Layers

**Bronze** — raw tables per source. One row per API object. Use source-native schema and IDs. Naming: `{source}_{entity}`.

Evidence: `docs/CONNECTORS_REFERENCE.md:10–18`

**Silver step 1** — `class_{domain}` tables, unified schema, source-native user IDs still present. Produced by the cross-source unification job.

Evidence: `docs/CONNECTORS_REFERENCE.md:20–27`

**Silver step 2** — same `class_{domain}` table names, same unified schema, but `person_id` replaces source-native user IDs. Produced by a separate identity resolution job.

Evidence: `docs/CONNECTORS_REFERENCE.md:28–33`

**Gold** — derived metrics, no raw events. Use domain-specific names without layer prefix — e.g. `status_periods`, `throughput`, `wip_snapshots`.

Evidence: `docs/CONNECTORS_REFERENCE.md:35–40`

## Identity Resolution

The Identity Manager is a PostgreSQL/MariaDB service that maps source-native user identifiers to a canonical `person_id`.

Sources for identity: email, username, employee_id, git login, and similar fields collected by HR connectors.

HR connectors (BambooHR, Workday, LDAP/AD) feed the Identity Manager directly alongside their Bronze tables.

Evidence: `docs/CONNECTORS_REFERENCE.md:22–26` — Identity Manager diagram.

## Source Categories

Seven source categories currently defined:

| Category | Examples |
|----------|---------|
| Version Control | GitHub, Bitbucket, GitLab |
| Task Tracking | YouTrack, Jira |
| Communication | Microsoft 365, Zulip |
| AI Dev Tool | Cursor, Windsurf, GitHub Copilot |
| AI Tool | Claude API, Claude Team, OpenAI API, ChatGPT Team |
| HR | BambooHR, Workday, LDAP/AD |
| CRM | HubSpot, Salesforce |
| Quality / Testing | Allure TestOps |

Evidence: section headings throughout `docs/CONNECTORS_REFERENCE.md`.

## Collection Run Tables

Every source has exactly one `{source}_collection_runs` table as its final Bronze table. This is a monitoring table only — never an analytics source.

Fields: `run_id`, `started_at`, `completed_at`, `status` (`running`/`completed`/`failed`), counts per entity type, `api_calls`, `errors`, `settings`.

Evidence: `docs/CONNECTORS_REFERENCE.md:333–347` — `github_collection_runs`.

## dbt Pipeline Conventions (summary)

Full specification: [docs/domain/ingestion-data-flow/specs/DESIGN.md](../../../docs/domain/ingestion-data-flow/specs/DESIGN.md) and ADRs in [docs/domain/ingestion-data-flow/specs/ADR/](../../../docs/domain/ingestion-data-flow/specs/ADR/).

**Hard rules** every dbt model under `src/ingestion/silver/` and `src/ingestion/connectors/*/dbt/` MUST follow:

1. **`engine='ReplacingMergeTree(_version)'`** for incremental models. Versionless `ReplacingMergeTree` only for `materialized='table'` with no `_version` column upstream. **Never** plain MergeTree.
2. **`order_by=['unique_key']`** — single column, never composite. Encode the natural key into `unique_key` in staging if needed.
3. **`unique_key` formula** — `{insight_tenant_id}-{insight_source_id}-{natural_key_parts}` everywhere (Airbyte AddFields, Python CDK helpers, SQL concat in explode models, Rust `format!`). See [ADR-0004](../../../docs/domain/ingestion-data-flow/specs/ADR/0004-unique-key-formula.md).
4. **Bronze tables MUST be promoted** to `ReplacingMergeTree(_airbyte_extracted_at)` on first dbt run via `promote_bronze_to_rmt` macro. Each connector has a `<connector>__bronze_promoted` bootstrap model. See [ADR-0002](../../../docs/domain/ingestion-data-flow/specs/ADR/0002-promote-bronze-to-rmt.md).
5. **Connector → silver via `union_by_tag`** — connectors write to per-connector staging models tagged `silver:<class>`; silver class models do `union_by_tag('silver:<class>')`. Never write directly to silver from a connector.
6. **Rust-owned staging tables** — wrap on the dbt side as `materialized='ephemeral'` (no DB object; dbt inlines as CTE). See [ADR-0003](../../../docs/domain/ingestion-data-flow/specs/ADR/0003-ephemeral-rust-passthrough.md).
7. **Read pattern** — silver consumers MUST use `SELECT … FROM silver.X FINAL` or `argMax(... ORDER BY _version)`. RMT tables hold multiple versions per `unique_key` until background merge.
8. **Airbyte sync mode** — always `destinationSyncMode='append'`. `append_dedup` and `overwrite` are forbidden (OOM, data loss on retry). See `cpt-dataflow-constraint-airbyte-append`.

### Anti-patterns

- ❌ Plain `MergeTree` (default engine) — duplicates accumulate forever
- ❌ Composite `order_by=(...)` instead of `['unique_key']`
- ❌ `materialized='view'` for silver `class_*` / `fct_*` / `mtr_*`
- ❌ `incremental_strategy='delete+insert'` with RMT — two dedup mechanisms stacked
- ❌ Reading silver without `FINAL` / `argMax`
- ❌ Connector emitting record without `unique_key` field

### Reference: shared macros

| Macro | Purpose |
|---|---|
| `union_by_tag(tag)` | Generates `UNION ALL` over all dbt models tagged with `tag`. Patched to handle ephemeral models (no DB relation check). |
| `promote_bronze_to_rmt(table, order_by)` | Idempotent migration of a bronze MergeTree to ReplacingMergeTree(_airbyte_extracted_at). |
| `create_task_field_history_staging()` | `on-run-start` macro: DDL of `staging.jira__task_field_history` (Rust-populated). |
| `snapshot()` | Append-only SCD2 helper. |
| `fields_history()` | Per-(entity, field) change log derived from a snapshot. |
| `identity_inputs_from_history()` | Emits UPSERT/DELETE observation rows for `identity.identity_inputs`. |

### Validation

- `cpt validate --artifact docs/domain/ingestion-data-flow/specs/DESIGN.md` — structure + cross-refs (audit trail)
- Skill `/check-dbt-conventions` — LLM-based correctness check (engine, order_by, unique_key formula presence)
- `dbt parse` — Jinja / config syntax (CI gate)

## Critical Files

| File | Why it matters |
|------|---------------|
| `docs/CONNECTORS_REFERENCE.md` | Single source of truth for all connector schemas, Bronze/Silver/Gold naming conventions, and the Identity Manager pipeline |
