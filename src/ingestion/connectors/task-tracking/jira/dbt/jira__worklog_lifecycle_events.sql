-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    alias='jira__worklog_lifecycle_events',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'silver', 'silver:class_task_field_history'],
    query_settings={'join_use_nulls': 1}
) }}

-- Worklog lifecycle as field-history events (specs/DELETION-AND-VISIBILITY.md):
-- add / set / remove enter silver.class_task_field_history like any other
-- change (field_id='worklog', event_kind='lifecycle'). The event carries the
-- worklog id — the lookup key into class_task_worklogs — not the payload.
-- add/set are dated by the worklog's own updated timestamp; remove by the
-- /worklog/deleted tombstone timestamp (real deletion time) when present,
-- detection time otherwise. Column order must match
-- staging.jira__task_field_history exactly: union_by_tag is positional.

WITH transitions AS (
    SELECT
        entity_id                                               AS worklog_id,
        tenant_id,
        source_id,
        multiIf(
            field_name = 'is_deleted' AND new_value = '1',          'remove',
            field_name = 'edited_at' AND old_value = '',            'add',
            field_name = 'edited_at',                               'set',
                                                                    ''
        )                                                       AS action,
        old_value,
        new_value,
        updated_at                                              AS detected_at
    FROM {{ ref('jira__worklog_lifecycle_history') }}
),

-- The identity timestamp is event_at, not detection time: detection time comes
-- from the snapshot's _tracked_at, which is second-resolution, so two
-- transitions of one worklog inside the same second would share unique_key and
-- collapse under ReplacingMergeTree. event_at carries the worklog's own
-- millisecond timestamp for add/set and the tombstone's for remove.
resolved AS (
    SELECT
        t.source_id                                             AS source_id,
        t.worklog_id                                            AS worklog_id,
        t.action                                                AS action,
        t.detected_at                                           AS detected_at,
        st.id_readable                                          AS id_readable,
        st.author_id                                            AS author_id,
        av.jira_id                                              AS jira_id,
        multiIf(
            t.action = 'remove' AND st.deleted_at_ms IS NOT NULL,
                fromUnixTimestamp64Milli(assumeNotNull(st.deleted_at_ms)),
            t.action IN ('add', 'set'),
                COALESCE(parseDateTime64BestEffortOrNull(t.new_value, 3),
                         toDateTime64(t.detected_at, 3)),
            toDateTime64(t.detected_at, 3)
        )                                                       AS event_at
    FROM transitions AS t
    LEFT JOIN {{ ref('jira__worklog_state') }} AS st FINAL
        ON st.tenant_id = t.tenant_id
        AND st.source_id = t.source_id
        AND st.worklog_id = t.worklog_id
    LEFT JOIN {{ ref('jira__issue_availability_state') }} AS av FINAL
        ON av.tenant_id = t.tenant_id
        AND av.source_id = t.source_id
        AND av.id_readable = st.id_readable
    WHERE t.action != ''
)

SELECT
    concat(COALESCE(t.source_id, ''), '-jira-worklog-', COALESCE(t.worklog_id, ''),
           '-', t.action, '-',
           toString(toUnixTimestamp64Milli(t.event_at)))         AS unique_key,
    COALESCE(t.source_id, '')                                   AS insight_source_id,
    CAST('jira' AS String)                                      AS data_source,
    COALESCE(t.jira_id, '')                                     AS issue_id,
    COALESCE(t.id_readable, '')                                 AS id_readable,
    CAST(NULL AS Nullable(String))                              AS title,
    concat('worklog:', COALESCE(t.worklog_id, ''), ':', t.action, ':',
           toString(toUnixTimestamp64Milli(t.event_at)))         AS event_id,
    t.event_at                                                  AS event_at,
    CAST('lifecycle', 'Enum8(\'changelog\' = 1, \'synthetic_initial\' = 2, \'availability\' = 3, \'lifecycle\' = 4)')
                                                                AS event_kind,
    toUInt32(0)                                                 AS _seq,
    t.author_id                                                 AS author_id,
    CAST(NULL AS Nullable(String))                              AS author_display,
    CAST('worklog' AS String)                                   AS field_id,
    CAST('Worklog' AS String)                                   AS field_name,
    CAST('single', 'Enum8(\'single\' = 1, \'multi\' = 2)')      AS field_cardinality,
    CAST(t.action, 'Enum8(\'set\' = 1, \'add\' = 2, \'remove\' = 3)') AS delta_action,
    toNullable(t.worklog_id)                                    AS delta_value_id,
    CAST(NULL AS Nullable(String))                              AS delta_value_display,
    CAST([COALESCE(t.worklog_id, '')] AS Array(String))         AS value_ids,
    CAST([COALESCE(t.worklog_id, '')] AS Array(String))         AS value_displays,
    CAST('opaque_id', 'Enum8(\'opaque_id\' = 1, \'account_id\' = 2, \'string_literal\' = 3, \'path\' = 4, \'none\' = 5)')
                                                                AS value_id_type,
    toDateTime64(t.detected_at, 3)                              AS collected_at,
    toUInt64(toUnixTimestamp64Milli(now64(3)))                  AS _version
FROM resolved AS t
