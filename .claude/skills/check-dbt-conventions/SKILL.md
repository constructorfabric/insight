---
name: check-dbt-conventions
description: "Audit dbt models and connector configurations against the data-flow conventions defined in docs/domain/ingestion-data-flow/specs/. Verifies engine=ReplacingMergeTree, order_by=['unique_key'], silver delete+insert, read-time dedup of every RMT read (FINAL/QUALIFY/LIMIT 1 BY/union_by_tag), unique_key formula, bronze→RMT promotion, ephemeral usage for Rust-owned tables, and Airbyte append-only sync mode. Reports deviations with file paths and line numbers."
disable-model-invocation: false
user-invocable: true
allowed-tools: Bash, Read, Glob, Grep
---

# Check dbt Pipeline Conventions

This skill audits the codebase against the four ADRs in `docs/domain/ingestion-data-flow/specs/ADR/` plus the DESIGN. It is **LLM-based correctness validation** — complements `cfs validate` which only checks artifact structure and code-marker presence (not engine config content).

## Source of truth

Before reporting anything, **read these specs THIS turn**:

1. [docs/domain/ingestion-data-flow/specs/DESIGN.md](../../../docs/domain/ingestion-data-flow/specs/DESIGN.md) — the convention document
2. [ADR-0001](../../../docs/domain/ingestion-data-flow/specs/ADR/0001-rmt-with-version-and-unique-key.md) — RMT(_version) + ORDER BY (unique_key)
3. [ADR-0002](../../../docs/domain/ingestion-data-flow/specs/ADR/0002-promote-bronze-to-rmt.md) — bronze MT → RMT promotion
4. [ADR-0003](../../../docs/domain/ingestion-data-flow/specs/ADR/0003-ephemeral-rust-passthrough.md) — ephemeral for Rust-written staging
5. [ADR-0004](../../../docs/domain/ingestion-data-flow/specs/ADR/0004-unique-key-formula.md) — unique_key formula

If specs change, this skill's checks update automatically — always derive the rules from the specs, not from this file.

## What to check

### Check 1 — Silver models: engine + order_by + write strategy

For every `.sql` file under `src/ingestion/silver/` (excluding `crm.disabled`):

- `engine` must be `'ReplacingMergeTree(_version)'` OR `'ReplacingMergeTree'` (versionless, only for `materialized='table'`)
- `order_by` must be `['unique_key']`
- `materialized` must NOT be `'view'` (per DESIGN §2.1: views forbidden for silver)
- If `materialized='incremental'` → `incremental_strategy='delete+insert'` AND `unique_key='unique_key'`
  (per ADR-0001 2026-06-03 amendment: silver is physically deduplicated so any
  consumer — gold views, the product — can read silver WITHOUT `FINAL`).
  `incremental_strategy='append'` in a silver model is now a FAIL.

Bash discovery:
```bash
find src/ingestion/silver -name "*.sql" -not -path "*disabled*"
```

For each file: read the `{{ config(...) }}` block. Report violations with file path + the offending line.

### Check 2 — Connector staging models: engine + order_by + unique_key projection

For every `.sql` file under `src/ingestion/connectors/*/dbt/`:

- If `materialized` is `incremental` or `table` → `engine='ReplacingMergeTree(_version)'` + `order_by=['unique_key']`
- If `materialized` is `view` → confirm it's a thin pass-through (no GROUP BY / window) AND the bronze upstream has been promoted (per ADR-0002)
- If `materialized` is `ephemeral` → confirm it's a pass-through over a Rust-written staging table (currently only `jira__task_field_history.sql`)
- The SELECT body MUST project a `unique_key` column (either propagated from bronze: `u.unique_key AS unique_key`, or computed: `CAST(concat(...) AS String) AS unique_key`)

Bash discovery:
```bash
find src/ingestion/connectors -name "*.sql" -path "*/dbt/*" -not -path "*disabled*"
```

### Check 3 — `unique_key` formula in connector record producers

For every Airbyte YAML connector at `src/ingestion/connectors/*/connector.yaml`:

- Find every `AddFields` block with `path: [unique_key]` (or `path: [unique]` — that's a deviation)
- Formula MUST start with `{{ config['insight_tenant_id'] }}-{{ config['insight_source_id'] }}-`
- Connector-specific tail follows (e.g., `{{ record['id'] }}` or composite)

For every Python CDK connector at `src/ingestion/connectors/*/source_*/`:

- Check `streams/base.py` (or equivalent) for a `_make_unique_key` helper
- Helper must include `tenant_id` and `source_id` as the first two arguments
- Helper output must contain those two before the natural key parts

Known deviation: claude-admin uses field name `unique` instead of `unique_key` and omits `tenant`/`source` prefix. Tracked as follow-up — flag it but don't double-report.

### Check 4 — `promote_bronze_to_rmt` calls in bootstrap models

For every `<connector>__bronze_promoted.sql` under `src/ingestion/connectors/*/dbt/`:

- Body must call `promote_bronze_to_rmt(table=..., order_by='unique_key')` for each bronze table the connector has (every table in its `sources:` block)
- All other connector models that read bronze must declare `-- depends_on: {{ ref('<connector>__bronze_promoted') }}`

**Every connector MUST have a `<connector>__bronze_promoted.sql`** — a missing one is a FAIL.
Known exceptions: legacy `git/github` (superseded by `github-v2`) is intentionally
not promoted; `claude-admin` is a tracked follow-up (its bronze lacks a `unique_key`
column, so promotion is blocked until the connector emits one — flag, don't double-report).

### Check 5 — Airbyte sync mode in connect.sh

- `src/ingestion/airbyte-toolkit/connect.sh` must have `dest_sync_mode = "append"` literal
- Must NOT have `append_dedup` or `overwrite` anywhere

### Check 6 — Ephemeral wrapping for Rust-owned staging

- Any model materialized as `ephemeral` must SELECT only from `source(...)` (not `ref(...)`) — i.e., it's a thin wrapper for a non-dbt-managed table
- The `union_by_tag` macro must contain the ephemeral handling branch (check `src/ingestion/dbt/macros/union_by_tag.sql` for `materialized == 'ephemeral'`)

### Check 7 — Incremental by default for silver/staging

Per `cpt-dataflow-principle-incremental-default` (DESIGN §2.1): every silver model and every dbt-owned staging model with append/event semantics MUST be `materialized='incremental'`. `materialized='table'` is allowed ONLY in three justified cases:

1. **Full-refresh source + current-state semantics**: small lookup/dimension tables whose upstream is a full-refresh API.
   - Allowed: `class_people`, `class_hr_working_hours`
2. **Aggregation that scans all data**: heavy GROUP BY / multi-CTE join requiring full rebuild.
   - Allowed: `mtr_git_person_totals`, `mtr_git_person_weekly`
3. **Explode/fan-out staging over full-refresh bronze**: one bronze row → many output rows AND bronze is full-refresh + overwrite.
   - Allowed: `jira__changelog_items`, `jira__issue_field_snapshot`

For each `.sql` file under `src/ingestion/silver/` and `src/ingestion/connectors/*/dbt/`:

- If `materialized='table'` AND model name is in the allow-list above → PASS
- If `materialized='table'` AND model name is NOT in the allow-list → FAIL with suggestion: "Convert to `materialized='incremental'`. For **silver** use `incremental_strategy='delete+insert'` + `unique_key='unique_key'`; for **staging** use `incremental_strategy='append'`. Add `WHERE _version > (SELECT max(_version) FROM {{ this }})`. If upstream lacks `_version`, amend the SELECT to project `toUnixTimestamp64Milli(_airbyte_extracted_at) AS _version`."
- If `materialized='view'` for a silver `class_*` / `fct_*` / `mtr_*` → FAIL (views forbidden in silver per check 1)
- If `materialized='ephemeral'` → cross-checked by Check 6

Bash discovery:
```bash
grep -lE "materialized\s*=\s*'table'" src/ingestion/silver/ src/ingestion/connectors/*/dbt/ -r 2>/dev/null
```

Cross-reference each match against the allow-list and report PASS / FAIL.

### Check 8 — Read-time deduplication of every RMT read

Per ADR-0001 (incl. the 2026-06-03 amendment): RMT only collapses duplicates on
background merge (never guaranteed at query time), so **every read of an RMT
relation must be deduplicated at read time** unless the producer is physically
unique. A duplicate that slips through inflates metrics (e.g. the
`slack_active_days = 42` incident).

A read is OK if ANY of:
- it goes through the **`union_by_tag`** macro (which dedups the union by
  `unique_key` via `QUALIFY ROW_NUMBER() … ORDER BY _version DESC`, or `LIMIT 1 BY`
  for versionless), OR
- the read carries `FINAL` / `argMax` / `QUALIFY ROW_NUMBER` / `LIMIT 1 BY` in its
  own subquery scope, OR
- the **producer is `incremental_strategy='delete+insert'` or `materialized='table'`**
  (physically unique → reader needs no dedup).

FAIL conditions (scan `connectors/*/dbt/` + `silver/` + gold migrations
`scripts/migrations/*.sql`):
1. A direct `ref()`/`source()` read of an RMT producer (append/incremental) with
   **no** dedup in scope and **not** via `union_by_tag`.
2. **Aggregation over un-deduplicated bronze**: `count()`/`sum()`/`avg()` over a
   `source('bronze_*')` read without `FINAL` (RMT bronze) or `LIMIT 1 BY unique_key`
   (non-promoted MergeTree bronze). Inflation is baked into one output row and is
   unrecoverable downstream (e.g. `bitbucket_cloud__commits`, `claude_admin__ai_dev_usage`).
3. A `snapshot()` macro call whose source is read without `FINAL` (spurious SCD2
   versions). The shared `macros/snapshot.sql` must read `FROM {{ source_ref }} FINAL`.
4. A **gold view** in `scripts/migrations/*.sql` reading a `staging.*`/`silver.*`
   append+RMT table without `FINAL`. (Reads of `delete+insert` silver are OK.
   Note migrations are immutable history — verify against the latest migration
   that (re)defines the view.)

**Deterministic complement**: `src/ingestion/dbt/audit_rmt_read_dedup.py` performs
this scan mechanically and exits non-zero on a gap — runnable in CI. Use it to
generate candidates, then confirm each by reading (it cannot resolve outer-scope
dedup or column-level semantics).

## How to run

When user invokes `/check-dbt-conventions`:

1. Read all 5 spec docs (`DESIGN.md` + 4 ADRs) THIS turn — confirm understanding
2. Run the 8 checks systematically using `Glob` / `Grep` / `Read` (optionally run `python3 src/ingestion/dbt/audit_rmt_read_dedup.py` for Check 8)
3. Report per-check status: PASS / FAIL with file paths + line numbers + the offending text
4. Summarize: total checks, failures by category
5. For each failure, suggest the minimal fix (e.g., "change `order_by='(insight_source_id, comment_id)'` to `order_by=['unique_key']`")

## Output format

```
=== check-dbt-conventions ===
Specs read: DESIGN.md, ADR-0001..ADR-0004 (this turn)

Check 1 — Silver engine + order_by — PASS (34 models, 0 violations)
Check 2 — Connector staging engine/order_by/unique_key — FAIL (2 violations)
  - src/ingestion/connectors/.../foo.sql:8 — order_by='(...)' should be ['unique_key']
  - src/ingestion/connectors/.../bar.sql:14 — missing unique_key projection in SELECT
Check 3 — unique_key formula — FAIL (8 violations in claude-admin — tracked)
Check 4 — promote_bronze_to_rmt bootstrap — PASS (1 connector covered: jira)
Check 5 — Airbyte append-only — PASS
Check 6 — Ephemeral wrapping — PASS
Check 7 — Incremental-by-default — PASS
Check 8 — Read-time dedup of RMT reads — FAIL (1 violation)
  - src/ingestion/connectors/.../foo.sql:42 — count()/sum() over source('bronze_*') without FINAL/LIMIT 1 BY

Summary: 6/8 PASS, 2 FAIL (Checks 2, 8 — fixes above), 1 known-tracked (Check 3 — claude-admin)
```

Refuse to report PASS without having read each spec doc this turn (anti-pattern: stale reasoning). Refuse to invent file paths — only report files actually scanned.

## When to invoke

User-driven, on demand:
- Before merging a connector PR
- After refactoring silver models
- Periodically (CI) — run before `dbt build` to surface convention drift
