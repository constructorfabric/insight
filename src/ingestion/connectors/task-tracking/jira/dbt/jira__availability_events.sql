-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__availability_events',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'silver', 'silver:class_task_field_history'],
    query_settings={'join_use_nulls': 1}
) }}

-- Availability transitions as synthetic field-history events
-- (specs/DELETION-AND-VISIBILITY.md). Deletion, lost access and re-appearance
-- enter silver.class_task_field_history exactly like any other field change
-- (field_id = 'availability', event_kind = 'availability'), so the entire
-- lifecycle of an issue — including how it disappeared — lives in ONE
-- source-agnostic table. How absence is DETECTED is a per-connector detail
-- (here: the census streams); other task trackers emit the same events from
-- whatever deletion signal their API exposes.
--
-- Column order must match staging.jira__task_field_history exactly:
-- union_by_tag concatenates the arms positionally.

SELECT
    concat(COALESCE(h.source_id, ''), '-jira-', COALESCE(h.entity_id, ''),
           '-availability-', event_id)                          AS unique_key,
    COALESCE(h.source_id, '')                                   AS insight_source_id,
    CAST('jira' AS String)                                      AS data_source,
    COALESCE(h.entity_id, '')                                   AS issue_id,
    COALESCE(st.id_readable, '')                                AS id_readable,
    CAST(NULL AS Nullable(String))                              AS title,
    event_id                                                    AS event_id,
    toDateTime64(h.updated_at, 3)                               AS event_at,
    CAST('availability', 'Enum8(\'changelog\' = 1, \'synthetic_initial\' = 2, \'availability\' = 3, \'lifecycle\' = 4)')
                                                                AS event_kind,
    toUInt32(0)                                                 AS _seq,
    CAST(NULL AS Nullable(String))                              AS author_id,
    CAST(NULL AS Nullable(String))                              AS author_display,
    CAST('availability' AS String)                              AS field_id,
    CAST('Availability' AS String)                              AS field_name,
    CAST('single', 'Enum8(\'single\' = 1, \'multi\' = 2)')      AS field_cardinality,
    CAST('set', 'Enum8(\'set\' = 1, \'add\' = 2, \'remove\' = 3)') AS delta_action,
    CAST(NULL AS Nullable(String))                              AS delta_value_id,
    toNullable(h.new_value)                                     AS delta_value_display,
    CAST([h.new_value] AS Array(String))                        AS value_ids,
    CAST([h.new_value] AS Array(String))                        AS value_displays,
    CAST('string_literal', 'Enum8(\'opaque_id\' = 1, \'account_id\' = 2, \'string_literal\' = 3, \'path\' = 4, \'none\' = 5)')
                                                                AS value_id_type,
    toDateTime64(h.updated_at, 3)                               AS collected_at,
    -- UInt64 like the Rust-written staging table; the union arms must agree.
    toUInt64(toUnixTimestamp64Milli(now64(3)))                  AS _version
FROM (
    -- event_id carries the issue id: the ADR-005 audit grain is
    -- (insight_source_id, data_source, id_readable, field_id, event_id), and
    -- census-only issues have an empty id_readable — without the issue id in
    -- event_id, every detection of one run would collapse into one grain.
    SELECT
        *,
        concat('availability:', COALESCE(entity_id, ''), ':',
               toString(toUnixTimestamp64Milli(toDateTime64(updated_at, 3)))) AS event_id
    FROM {{ ref('jira__issue_availability_history') }}
    WHERE field_name = 'availability'
) AS h
LEFT JOIN {{ ref('jira__issue_availability_state') }} AS st FINAL
    ON st.tenant_id = h.tenant_id
    AND st.source_id = h.source_id
    AND st.jira_id = h.entity_id
