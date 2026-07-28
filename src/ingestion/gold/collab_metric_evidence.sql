{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'source_key', 'measure_key', 'entity_id', 'metric_date', 'record_id'],
    schema='insight',
    alias='collab_metric_evidence',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
) }}


WITH
chat_source AS (
    SELECT
        tenant_id,
        person_key AS entity_id,
        date AS metric_date,
        total_chat_messages,
        channel_posts,
        channel_replies,
        direct_and_group_messages,
        replaceOne(data_source, 'insight_', '') AS tool_value,
        {{ collab_tool_label('tool_value', m365_label='Microsoft Teams') }} AS tool_label,
        CAST(
            [tuple('tool', tool_value, tool_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions
    FROM {{ ref('class_collab_chat_activity') }} FINAL
    WHERE person_key LIKE '%@%'
      AND date IS NOT NULL
),
meeting_source AS (
    SELECT
        tenant_id,
        person_key AS entity_id,
        date AS metric_date,
        meetings_attended,
        meetings_organized,
        adhoc_meetings_attended,
        scheduled_meetings_attended,
        audio_duration_seconds,
        video_duration_seconds,
        screen_share_duration_seconds,
        replaceOne(data_source, 'insight_', '') AS tool_value,
        {{ collab_tool_label('tool_value', m365_label='Microsoft Teams') }} AS tool_label,
        CAST(
            [tuple('tool', tool_value, tool_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions
    FROM {{ ref('class_collab_meeting_activity') }} FINAL
    WHERE person_key LIKE '%@%'
      AND date IS NOT NULL
),
email_source AS (
    SELECT
        tenant_id,
        person_key AS entity_id,
        date AS metric_date,
        sent_count,
        received_count,
        read_count,
        replaceOne(data_source, 'insight_', '') AS tool_value,
        {{ collab_tool_label('tool_value') }} AS tool_label,
        CAST(
            [tuple('tool', tool_value, tool_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions
    FROM {{ ref('class_collab_email_activity') }} FINAL
    WHERE person_key LIKE '%@%'
      AND date IS NOT NULL
),
document_source AS (
    SELECT
        tenant_id,
        person_key AS entity_id,
        date AS metric_date,
        viewed_or_edited_count,
        shared_internally_count,
        shared_externally_count,
        replaceOne(data_source, 'insight_', '') AS tool_value,
        {{ collab_tool_label('tool_value') }} AS tool_label,
        CAST(
            [tuple('tool', tool_value, tool_label)]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions,
        CAST(
            [tuple('scope', 'internal', 'Internal')]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS internal_scope_dimensions,
        CAST(
            [tuple('scope', 'external', 'External')]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS external_scope_dimensions
    FROM {{ ref('class_collab_document_activity') }} FINAL
    WHERE person_key LIKE '%@%'
      AND date IS NOT NULL
),
focus_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        email AS entity_id,
        day AS metric_date,
        dev_time_h,
        working_hours_per_day,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM {{ ref('class_focus_metrics') }} FINAL
    WHERE email LIKE '%@%'
      AND day IS NOT NULL
),
deliberate_activity AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        tool_value,
        modality,
        CAST(
            [tuple('tool', tool_value, {{ collab_tool_label('tool_value') }})]
            AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS tool_dimensions,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM (
        SELECT DISTINCT tenant_id, entity_id, metric_date, tool_value, modality
        FROM (
            SELECT tenant_id, entity_id, metric_date, tool_value, 'chat' AS modality
            FROM chat_source
            WHERE total_chat_messages > 0
            UNION ALL
            SELECT tenant_id, entity_id, metric_date, tool_value, 'email' AS modality
            FROM email_source
            WHERE sent_count > 0
            UNION ALL
            SELECT tenant_id, entity_id, metric_date, tool_value, 'documents' AS modality
            FROM document_source
            WHERE viewed_or_edited_count > 0
               OR shared_internally_count > 0
               OR shared_externally_count > 0
            UNION ALL
            SELECT tenant_id, entity_id, metric_date, tool_value, 'meetings' AS modality
            FROM meeting_source
            WHERE meetings_attended > 0
        )
    )
),
meeting_free_source AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        if(sum(meeting_seconds) = 0, 1, 0) AS meeting_free_flag,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM (
        SELECT DISTINCT
            tenant_id,
            entity_id,
            metric_date,
            0 AS meeting_seconds,
            1 AS is_active
        FROM deliberate_activity
        UNION ALL
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            ifNull(audio_duration_seconds, 0)
                + ifNull(video_duration_seconds, 0)
                + ifNull(screen_share_duration_seconds, 0) AS meeting_seconds,
            0 AS is_active
        FROM meeting_source
    )
    GROUP BY tenant_id, entity_id, metric_date
    HAVING max(is_active) = 1
),
value_measures AS (
    {{ sum_measure('total_chat_messages', 'chat_source', 'total_chat_messages', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('channel_posts', 'chat_source', 'channel_posts + ifNull(channel_replies, 0)', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('direct_and_group_messages', 'chat_source', 'direct_and_group_messages', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('emails_sent', 'email_source', 'sent_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('emails_received', 'email_source', 'received_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('emails_read', 'email_source', 'read_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('files_engaged', 'document_source', 'viewed_or_edited_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('files_shared_internal', 'document_source', 'shared_internally_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('files_shared_external', 'document_source', 'shared_externally_count', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('files_shared', 'document_source', 'shared_internally_count', 'internal_scope_dimensions') }}

    UNION ALL

    {{ sum_measure('files_shared', 'document_source', 'shared_externally_count', 'external_scope_dimensions') }}

    UNION ALL

    {{ sum_measure('meeting_hours', 'meeting_source', 'greatest(ifNull(audio_duration_seconds, 0), ifNull(video_duration_seconds, 0), ifNull(screen_share_duration_seconds, 0)) / 3600.0', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('meetings_attended', 'meeting_source', 'meetings_attended', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('meetings_organized', 'meeting_source', 'meetings_organized', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('adhoc_meetings_attended', 'meeting_source', 'adhoc_meetings_attended', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('scheduled_meetings_attended', 'meeting_source', 'scheduled_meetings_attended', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('focus_hours', 'focus_source', 'dev_time_h', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('working_hours', 'focus_source', 'working_hours_per_day', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('chat_active_day', 'chat_source', 'if(total_chat_messages > 0, 1, NULL)', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('meeting_free_day', 'meeting_free_source', 'meeting_free_flag', 'no_dimensions') }}
),
active_day_grain AS (
    SELECT DISTINCT
        tenant_id,
        entity_id,
        metric_date,
        tool_dimensions
    FROM deliberate_activity
),
active_modality_grain AS (
    SELECT DISTINCT
        tenant_id,
        entity_id,
        metric_date,
        modality,
        no_dimensions
    FROM deliberate_activity
),
subject_measures AS (
    {{ distinct_measure('active_day', 'active_day_grain', 'metric_date', 'tool_dimensions') }}

    UNION ALL

    {{ distinct_measure('active_modality', 'active_modality_grain', 'modality', 'no_dimensions') }}
)
SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'collab' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    concat(
        toString(metric_date),
        ':',
        measure_key,
        ':',
        hex(sipHash128(toString(arrayMap(d -> tuple(d.1, d.2), dimensions))))
    ) AS record_id,
    measure_key AS record_kind,
    if(
        measure_key IN ('focus_hours', 'working_hours', 'meeting_free_day'),
        'derived_population',
        'source_summary'
    ) AS granularity,
    replaceAll(measure_key, '_', ' ') AS record_label,
    value AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM value_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'collab' AS source_key,
    'person' AS entity_type,
    assumeNotNull(entity_id) AS entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    concat(
        toString(metric_date),
        ':',
        measure_key,
        ':',
        hex(sipHash128(toString(tuple(arrayMap(d -> tuple(d.1, d.2), dimensions), subject_key))))
    ) AS record_id,
    measure_key AS record_kind,
    'derived_population' AS granularity,
    replaceAll(measure_key, '_', ' ') AS record_label,
    value AS contribution,
    subject_key,
    dimensions,
    CAST(map() AS Map(String, String)) AS details
FROM subject_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
