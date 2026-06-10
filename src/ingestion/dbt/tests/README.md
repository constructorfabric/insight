# Data-quality checks

Each check is a singular dbt test: a SQL file that selects the **violating**
rows. A check passes when it returns zero rows. Checks cover silver tables and
gold views alike — gold tests read the `insight` views through the registered
`gold` source.

A scheduled run executes the opt-in catalog — the tests tagged `data_quality`
(`dbt test --selector data_quality`, defined in `selectors.yml`). For every
check, an `on-run-end` hook
(`macros/emit_dq_findings.sql`) prints one JSON `DQ_FINDING` line to stdout for
the central log store, and `store_failures` keeps the violating rows in an
audit table for drill-down. Checks are non-blocking (`severity=warn`), so a
finding never fails the pipeline. Untagged tests (including the generic
`not_null`/`unique`/`relationships` assertions) keep dbt's default `error`
severity and run under `dbt build` for build integrity; they are not part of
the scheduled run and don't emit findings.

## Adding a check

Create `tests/<domain>/assert_<subject>_<rule>.sql`:

```sql
{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Short human label',
        'domain': 'collab',            -- collab | git | task | ai | hr | gold | ...
        'category': 'physical_bound',  -- source_uniqueness | grain | physical_bound | freshness | ...
        'tier': 'error',               -- triage importance: info | warn | error
        'remediation': 'What to check / how to fix when this fires.'
    }
) }}
-- Select the rows that should not exist. One or more rows = a violation.
SELECT ...
FROM silver.class_...        -- or {{ source('gold', '<view>') }} for a gold view
WHERE <violating condition>
```

Conventions:

- **Tag it `data_quality`.** Only tagged tests run in the scheduled job. An
  untagged test is still a valid dbt test but won't be monitored.
- **Read silver/gold only — never bronze.** Silver/gold exist regardless of the
  connector set, so a check adapts to any tenant: a missing connector class is
  just an empty table and a clean pass. Bronze is per-connector and may be
  absent, which would make the check error. Checks that need bronze (e.g.
  silver-to-bronze traceability) are not data quality — leave them untagged;
  they run under `dbt build`.
- **No `LIMIT`.** The row count is the violation count; `store_failures` keeps
  every offending row, and samples are taken at read time from the audit table.
- **`severity`** is the build gate: `warn` (advisory, non-blocking) or `error`
  (blocks the run). Default to `warn`; use `error` only when a violation must
  stop the pipeline.
- **`meta.tier`** is independent of the gate — how alarming a violation is to a
  human, surfaced in the finding regardless of the gate.
- Register any new gold view in `silver/_shared/gold_sources.yml` before
  referencing it with `source('gold', '<view>')`.
- Read `ReplacingMergeTree` tables with `FINAL` (or the project's dedup pattern)
  so transient duplicates don't show up as false violations.

## Finding shape

```json
{
  "event": "data_quality_finding",
  "run_id": "<dbt invocation id>",
  "check_id": "assert_focus_metrics_working_hours_bound",
  "title": "Working hours per day within physical bounds",
  "domain": "hr", "category": "physical_bound",
  "gate": "warn", "tier": "warn", "status": "warn",
  "rows_violating": 559, "duration_ms": 78,
  "audit_relation": "silver_dbt_test__audit.assert_focus_metrics_working_hours_bound",
  "remediation": "..."
}
```
