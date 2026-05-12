{#-
  Creates `staging.jira__task_field_history` — the only task-tracking table that dbt
  does NOT own. It is written exclusively by the `jira-enrich` Rust binary (ADR-003).

  dbt then unions it into `silver.class_task_field_history` via
  `src/ingestion/silver/task-tracking/class_task_field_history.sql` (which uses
  `union_by_tag`). All other `silver.class_task_*` tables are materialized directly
  by dbt via `union_by_tag` over the connector-level staging models.

  Called from `on-run-start` so the table exists before enrich runs.
-#}

{% macro create_task_field_history_staging() %}
    {% do run_query("CREATE DATABASE IF NOT EXISTS silver") %}
    {% do run_query("CREATE DATABASE IF NOT EXISTS staging") %}

    {% do run_query("
        CREATE TABLE IF NOT EXISTS staging.jira__task_field_history
        (
            unique_key          String,
            insight_source_id   String,
            data_source         String,
            issue_id            String,
            id_readable         String,
            event_id            String,
            event_at            DateTime64(3),
            event_kind          Enum8('changelog' = 1, 'synthetic_initial' = 2),
            _seq                UInt32,
            author_id           Nullable(String),
            author_display      Nullable(String),
            field_id            String,
            field_name          String,
            field_cardinality   Enum8('single' = 1, 'multi' = 2),
            delta_action        Enum8('set' = 1, 'add' = 2, 'remove' = 3),
            delta_value_id      Nullable(String),
            delta_value_display Nullable(String),
            value_ids           Array(String),
            value_displays      Array(String),
            value_id_type       Enum8('opaque_id' = 1, 'account_id' = 2, 'string_literal' = 3, 'path' = 4, 'none' = 5),
            collected_at        DateTime64(3),
            _version            UInt64,

            INDEX idx_fh_issue    (insight_source_id, data_source, issue_id)    TYPE minmax GRANULARITY 4,
            INDEX idx_fh_readable (insight_source_id, data_source, id_readable) TYPE minmax GRANULARITY 4,
            INDEX idx_fh_event_at (event_at)                                    TYPE minmax GRANULARITY 4
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}

    {% if execute %}
        {{ log("Ensured staging.jira__task_field_history (DDL owned here; populated by jira-enrich)", info=True) }}
    {% endif %}
{% endmacro %}
