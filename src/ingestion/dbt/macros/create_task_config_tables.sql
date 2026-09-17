{#-
  Creates the operator-authored task-tracking configuration relations. dbt
  does NOT own their contents: an operator writes the rows, dbt only guarantees
  the tables exist and reads them as sources. A dbt-owned model would recreate
  them on every run and wipe the decisions they hold.

  They answer what no vendor can state. `task_field_roles` binds a vendor field
  identifier to the metric role gold consumes, so gold matches a role instead of
  a vendor's literal. `task_value_map` binds a vendor value identifier to the
  canonical value a class dimension carries. `field_value_map` is one generic
  mapping table for every standardized field: `field` names what is being
  standardized (currently `issue_type`), `source_key` is the vendor's key for
  the value, `target_value` is the canonical value (domain per field —
  issue_type: bug, task, unknown), and `display_name` records the vendor name
  the decision was made against, so a later rename of the vendor value is
  detectable rather than silent. `field_value_defaults` assigns, per
  (tenant, source, field), the value unmapped keys fall to; no default row means
  the hardcoded `unknown` terminal, never the raw source value.

  For issue types `source_key` is the vendor's type id on both sides:
  org-scoped for GitHub, per-project for Jira, so Jira needs one decision per
  project type. Keying on the name instead would let a rename re-classify
  closed history, silently and retroactively. An id with no row falls to the
  tenant's `field_value_defaults` row, then to `unknown`.

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

    {% do run_query("
        CREATE TABLE IF NOT EXISTS config.field_value_map
        (
            tenant_id         String,
            insight_source_id String,
            data_source       LowCardinality(String),
            field             LowCardinality(String),
            source_key        String,
            valid_from        DateTime64(3),
            recorded_at       DateTime64(3),
            unique_key        String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                                    data_source, ':', field, ':', source_key, ':',
                                                    toString(valid_from), ':', toString(recorded_at)),
            target_value      LowCardinality(String),
            display_name      String,
            is_deleted        UInt8  DEFAULT 0,
            note              String DEFAULT '',
            recorded_by       String DEFAULT '',
            _version          DateTime64(3) DEFAULT now64(3)
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}

    {% do run_query("
        CREATE TABLE IF NOT EXISTS config.field_value_defaults
        (
            tenant_id         String,
            insight_source_id String,
            field             LowCardinality(String),
            valid_from        DateTime64(3),
            recorded_at       DateTime64(3),
            unique_key        String DEFAULT concat(tenant_id, ':', insight_source_id, ':',
                                                    field, ':',
                                                    toString(valid_from), ':', toString(recorded_at)),
            default_value     LowCardinality(String),
            is_deleted        UInt8  DEFAULT 0,
            note              String DEFAULT '',
            recorded_by       String DEFAULT '',
            _version          DateTime64(3) DEFAULT now64(3)
        )
        ENGINE = ReplacingMergeTree(_version)
        ORDER BY (unique_key)
    ") %}

    {% if execute %}
        {{ log("Ensured config.task_field_roles, config.task_value_map, config.field_value_map and config.field_value_defaults (DDL owned here; rows authored by an operator)", info=True) }}
    {% endif %}
{% endmacro %}
