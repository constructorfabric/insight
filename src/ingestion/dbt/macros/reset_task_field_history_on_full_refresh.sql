{#-
  Pre-hook for `jira__changelog_items`: on a full refresh, empty the Rust-owned
  `staging.jira__task_field_history` so `jira-enrich` re-derives it from scratch.

  Without this, `--full-refresh` cannot reach that table at all: its DDL macro
  is CREATE TABLE IF NOT EXISTS, and jira-enrich keeps ONE high-water mark per
  issue across all fields, dropping every event at or below it. Any event that
  reaches Bronze late — a repaired row, a backfilled project — would stay
  invisible forever while the rest of the pipeline reported success.

  It hangs off `jira__changelog_items` (tags staging+jira) rather than
  on-run-start because the Jira pipeline invokes dbt twice, staging then silver,
  and passes --full-refresh to both. An on-run-start reset would fire again on
  the silver pass, wiping what enrich had just written. This model belongs to
  the staging selection only, so the reset lands exactly once, before enrich.
-#}

{% macro reset_task_field_history_on_full_refresh() %}
    {%- if flags.FULL_REFRESH -%}
        TRUNCATE TABLE IF EXISTS staging.jira__task_field_history
    {%- else -%}
        SELECT 1
    {%- endif -%}
{% endmacro %}
