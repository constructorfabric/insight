{#-
  Creates the two operator-authored task-tracking configuration relations. dbt
  does NOT own their contents: an operator writes the rows, dbt only guarantees
  the tables exist and reads them as sources. A dbt-owned model would recreate
  them on every run and wipe the decisions they hold.

  They answer what no vendor can state. `task_field_roles` binds a vendor field
  identifier to the metric role gold consumes, so gold matches a role instead of
  a vendor's literal. `task_value_map` binds a vendor value identifier to the
  canonical value a class dimension carries — a status category, an issue kind.

  Bitemporal by design. `valid_from` says which events a mapping applies to: the
  process genuinely changed on a date. `recorded_at` says when the decision was
  made: the mapping was wrong and history is being corrected. One axis cannot
  express both. A period's end is derived from the next period's start, so gaps
  and overlaps are unrepresentable and nothing has to be closed by hand.

  ReplacingMergeTree keyed on the whole decision — `recorded_at` is part of
  `unique_key`, so deduplication only ever collapses the same decision written
  twice (a re-run seed, a retried statement). A correction of an earlier
  decision is a different key and survives, which is what keeps the journal.

  Called from `on-run-start` so the tables exist before any model reads them.
-#}

{% macro create_task_config_tables() %}
    {% do run_query("CREATE DATABASE IF NOT EXISTS config") %}

    {% do run_query("
        CREATE TABLE IF NOT EXISTS config.task_field_roles
        (
            tenant_id         String,
            insight_source_id String,
            data_source       LowCardinality(String),
            field_id          String,
            valid_from        DateTime64(3),
            recorded_at       DateTime64(3),
            unique_key        String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                                    data_source, ':', field_id, ':',
                                                    toString(valid_from), ':', toString(recorded_at)),
            role              LowCardinality(String),
            precedence        UInt8   DEFAULT 0,
            value_unit        LowCardinality(String) DEFAULT 'none',
            unit_multiplier   Float64 DEFAULT 1,
            is_deleted        UInt8   DEFAULT 0,
            note              String  DEFAULT '',
            recorded_by       String  DEFAULT '',
            _version          DateTime64(3) DEFAULT now64(3)
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}

    {% do run_query("
        CREATE TABLE IF NOT EXISTS config.task_value_map
        (
            tenant_id         String,
            insight_source_id String,
            data_source       LowCardinality(String),
            field_id          String,
            value_id          String,
            valid_from        DateTime64(3),
            recorded_at       DateTime64(3),
            unique_key        String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                                    data_source, ':', field_id, ':', value_id, ':',
                                                    toString(valid_from), ':', toString(recorded_at)),
            canonical_value   LowCardinality(String),
            value_display     String DEFAULT '',
            is_deleted        UInt8  DEFAULT 0,
            note              String DEFAULT '',
            recorded_by       String DEFAULT '',
            _version          DateTime64(3) DEFAULT now64(3)
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}

    {% if execute %}
        {{ log("Ensured config.task_field_roles and config.task_value_map (DDL owned here; rows authored by an operator)", info=True) }}
    {% endif %}
{% endmacro %}
