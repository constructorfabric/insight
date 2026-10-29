{{ metric_evidence_table() }}

-- Keyed by the source identity, not by person: the analytics runtime resolves
-- through `identity.account_assignment` first and `identity.person_map` second
-- while it serves. An unresolvable row stays and starts counting the moment it
-- resolves. Every row is keyed by one `entity_id`: the normalized email where
-- the source reported one, else an opaque key naming the source account (a Zoom
-- guest known by display name only). The outer SELECT restores the account
-- triple beside that key; no email map carries the key, so the account path
-- decides.
WITH account_keys AS (
    SELECT DISTINCT
        tenant_id,
        entity_id,
        account_source_type,
        account_source_id,
        account_id
    FROM (
        SELECT
            tenant_id,
            {{ account_entity_key('account_source_type', 'account_source_id', 'account_id') }} AS entity_id,
            account_source_type,
            account_source_id,
            account_id
        FROM {{ ref('class_collab_meeting_activity') }} FINAL
        WHERE account_source_type != 'email'
    )
)
SELECT
    src.tenant_id,
    src.source_key,
    src.entity_type,
    if(coalesce(acc.account_id, '') != '', src.entity_id, {{ normalized_email('src.entity_id') }}) AS entity_id,
    -- '' on an email-keyed row leaves the account join unmatched.
    coalesce(acc.account_source_type, '') AS account_source_type,
    coalesce(acc.account_source_id, '') AS account_source_id,
    coalesce(acc.account_id, '') AS account_id,
    src.metric_date,
    src.observed_at,
    src.measure_key,
    src.record_id,
    src.record_kind,
    src.granularity,
    src.record_label,
    src.contribution,
    src.subject_key,
    src.dimensions,
    src.details
FROM (


WITH
chat_source AS (
    SELECT
        tenant_id,
        {{ normalized_email('person_key') }} AS entity_id,
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
        if(
            account_source_type = 'email',
            {{ normalized_email('person_key') }},
            {{ account_entity_key('account_source_type', 'account_source_id', 'account_id') }}
        ) AS entity_id,
        date AS metric_date,
        meetings_attended + ifNull(calls_count, 0) AS meetings_and_calls_attended,
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
    WHERE (account_source_type != 'email' OR person_key LIKE '%@%')
      AND date IS NOT NULL
),
email_source AS (
    SELECT
        tenant_id,
        {{ normalized_email('person_key') }} AS entity_id,
        date AS metric_date,
        sent_count,
        received_count,
        read_count,
        replaceOne(data_source, 'insight_', '') AS tool_value,
        {{ collab_tool_label('tool_value', m365_label='Outlook') }} AS tool_label,
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
        {{ normalized_email('person_key') }} AS entity_id,
        date AS metric_date,
        viewed_or_edited_count,
        shared_internally_count,
        shared_externally_count,
        replaceOne(data_source, 'insight_', '') AS tool_value,
        {{ collab_tool_label('tool_value', m365_label='OneDrive / SharePoint') }} AS tool_label,
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
working_hours_source AS (
    SELECT
        insight_tenant_id AS tenant_id,
        {{ normalized_email('email') }} AS entity_id,
        max(working_hours_per_day) AS working_hours_per_day
    FROM {{ ref('class_hr_working_hours') }} FINAL
    WHERE email LIKE '%@%'
    GROUP BY tenant_id, entity_id
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
            WHERE meetings_and_calls_attended > 0
        )
    )
),
meeting_covered_days AS (
    -- INVARIANT: an address no meeting source ever reported has unknown meeting time,
    -- so its days are neither meeting-free nor focus days.
    SELECT DISTINCT
        tenant_id,
        entity_id,
        metric_date
    FROM deliberate_activity
    WHERE (tenant_id, entity_id) IN (
        SELECT DISTINCT tenant_id, entity_id
        FROM meeting_source
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
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            0 AS meeting_seconds,
            1 AS is_active
        FROM meeting_covered_days
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
workday_source AS (
    SELECT
        active_days.tenant_id AS tenant_id,
        active_days.entity_id AS entity_id,
        active_days.metric_date AS metric_date,
        -- WORKAROUND: under join_use_nulls=0 an unmatched LEFT JOIN yields 0, not NULL.
        if(hours.working_hours_per_day > 0, hours.working_hours_per_day, 8.0) AS working_hours,
        least(ifNull(meetings.meeting_hours, 0), working_hours) AS workday_meeting_hours,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS no_dimensions
    FROM meeting_covered_days AS active_days
    LEFT JOIN (
        SELECT
            tenant_id,
            entity_id,
            metric_date,
            sum(greatest(
                ifNull(audio_duration_seconds, 0),
                ifNull(video_duration_seconds, 0),
                ifNull(screen_share_duration_seconds, 0)
            )) / 3600.0 AS meeting_hours
        FROM meeting_source
        GROUP BY tenant_id, entity_id, metric_date
    ) AS meetings
        ON meetings.tenant_id = active_days.tenant_id
       AND meetings.entity_id = active_days.entity_id
       AND meetings.metric_date = active_days.metric_date
    LEFT JOIN working_hours_source AS hours
        ON hours.tenant_id = active_days.tenant_id
       AND hours.entity_id = active_days.entity_id
),
value_measures AS (
    {{ sum_measure('total_chat_messages', 'chat_source', 'total_chat_messages', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('channel_posts', 'chat_source', 'channel_posts + ifNull(channel_replies, 0)', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('direct_and_group_messages', 'chat_source', 'direct_and_group_messages', 'tool_dimensions') }}

    UNION ALL

    -- INVARIANT: only tools that tell direct from channel messages apart feed this,
    -- so a total without that split never dilutes the direct share.
    {{ sum_measure('split_chat_messages', 'chat_source', 'if(direct_and_group_messages IS NULL, NULL, total_chat_messages)', 'tool_dimensions') }}

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

    {{ sum_measure('meetings_attended', 'meeting_source', 'meetings_and_calls_attended', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('meetings_organized', 'meeting_source', 'meetings_organized', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('adhoc_meetings_attended', 'meeting_source', 'adhoc_meetings_attended', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('scheduled_meetings_attended', 'meeting_source', 'scheduled_meetings_attended', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('workday_meeting_hours', 'workday_source', 'workday_meeting_hours', 'no_dimensions') }}

    UNION ALL

    {{ sum_measure('working_hours', 'workday_source', 'working_hours', 'no_dimensions') }}

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
chat_active_day_grain AS (
    SELECT DISTINCT
        tenant_id,
        entity_id,
        metric_date,
        tool_dimensions
    FROM chat_source
    WHERE total_chat_messages > 0
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

    UNION ALL

    {{ distinct_measure('chat_active_day', 'chat_active_day_grain', 'metric_date', 'tool_dimensions') }}
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
        measure_key IN ('workday_meeting_hours', 'working_hours', 'meeting_free_day'),
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
) AS src
LEFT JOIN account_keys AS acc
    ON acc.tenant_id = src.tenant_id
   AND acc.entity_id = src.entity_id
