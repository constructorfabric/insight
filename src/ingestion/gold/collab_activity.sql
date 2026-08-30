{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'person_email', 'activity_date'],
    partition_by='toYYYYMM(activity_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Collaboration work per person, day and tool: the chat, meeting, email and
-- document facts each tool reported, the focus hours the day was measured at,
-- and the flags saying which modalities the person was deliberately active in.
--
-- INVARIANT: identity is the person email the source recorded, normalized and
-- nothing more; the person is bound when a query runs.

WITH
chat_activity AS (
    SELECT
        tenant_id,
        {{ normalized_email('person_key') }} AS person_email,
        date AS activity_date,
        replaceOne(data_source, 'insight_', '') AS tool,
        total_chat_messages,
        channel_posts + ifNull(channel_replies, 0) AS channel_posts_total,
        direct_and_group_messages,
        toUInt8(total_chat_messages > 0) AS is_chat_active
    FROM {{ ref('class_collab_chat_activity') }} FINAL
    WHERE tenant_id IS NOT NULL
      AND person_key LIKE '%@%'
      AND date IS NOT NULL
),
meeting_activity AS (
    SELECT
        tenant_id,
        {{ normalized_email('person_key') }} AS person_email,
        date AS activity_date,
        replaceOne(data_source, 'insight_', '') AS tool,
        meetings_attended,
        meetings_organized,
        adhoc_meetings_attended,
        scheduled_meetings_attended,
        -- SAFETY: `greatest` ignores NULL arguments, so every duration is
        -- grounded first — a meeting with no reported duration is zero time,
        -- not an unknown one.
        ifNull(audio_duration_seconds, 0)
            + ifNull(video_duration_seconds, 0)
            + ifNull(screen_share_duration_seconds, 0) AS meeting_seconds,
        greatest(
            ifNull(audio_duration_seconds, 0),
            ifNull(video_duration_seconds, 0),
            ifNull(screen_share_duration_seconds, 0)
        ) / 3600.0 AS meeting_hours,
        toUInt8(meetings_attended > 0) AS is_meetings_active
    FROM {{ ref('class_collab_meeting_activity') }} FINAL
    WHERE tenant_id IS NOT NULL
      AND person_key LIKE '%@%'
      AND date IS NOT NULL
),
email_activity AS (
    SELECT
        tenant_id,
        {{ normalized_email('person_key') }} AS person_email,
        date AS activity_date,
        replaceOne(data_source, 'insight_', '') AS tool,
        CAST(sent_count AS Nullable(Float64)) AS emails_sent,
        CAST(received_count AS Nullable(Float64)) AS emails_received,
        CAST(read_count AS Nullable(Float64)) AS emails_read,
        toUInt8(ifNull(sent_count, 0) > 0) AS is_email_active
    FROM {{ ref('class_collab_email_activity') }} FINAL
    WHERE tenant_id IS NOT NULL
      AND person_key LIKE '%@%'
      AND date IS NOT NULL
),
document_activity AS (
    SELECT
        tenant_id,
        {{ normalized_email('person_key') }} AS person_email,
        date AS activity_date,
        replaceOne(data_source, 'insight_', '') AS tool,
        CAST(viewed_or_edited_count AS Nullable(Float64)) AS files_engaged,
        CAST(shared_internally_count AS Nullable(Float64)) AS files_shared_internal,
        CAST(shared_externally_count AS Nullable(Float64)) AS files_shared_external,
        toUInt8(
            ifNull(viewed_or_edited_count, 0) > 0
            OR ifNull(shared_internally_count, 0) > 0
            OR ifNull(shared_externally_count, 0) > 0
        ) AS is_documents_active
    FROM {{ ref('class_collab_document_activity') }} FINAL
    WHERE tenant_id IS NOT NULL
      AND person_key LIKE '%@%'
      AND date IS NOT NULL
),
-- Focus hours are a property of the day, not of any one tool, so they ride the
-- day's lowest-named tool row. Fanning them onto every tool row of the day
-- would multiply them by the tool count.
focus_carrier_tool AS (
    SELECT
        tenant_id,
        person_email,
        activity_date,
        min(tool) AS tool
    FROM (
        SELECT tenant_id, person_email, activity_date, tool FROM chat_activity
        UNION ALL
        SELECT tenant_id, person_email, activity_date, tool FROM meeting_activity
        UNION ALL
        SELECT tenant_id, person_email, activity_date, tool FROM email_activity
        UNION ALL
        SELECT tenant_id, person_email, activity_date, tool FROM document_activity
    )
    GROUP BY tenant_id, person_email, activity_date
),
focus_activity AS (
    SELECT
        carrier.tenant_id AS tenant_id,
        carrier.person_email AS person_email,
        carrier.activity_date AS activity_date,
        carrier.tool AS tool,
        focus.focus_hours AS focus_hours,
        focus.working_hours AS working_hours
    FROM (
        SELECT
            insight_tenant_id AS tenant_id,
            {{ normalized_email('email') }} AS person_email,
            day AS activity_date,
            dev_time_h AS focus_hours,
            working_hours_per_day AS working_hours
        FROM {{ ref('class_focus_metrics') }} FINAL
        WHERE insight_tenant_id IS NOT NULL
          AND email LIKE '%@%'
          AND day IS NOT NULL
    ) AS focus
    INNER JOIN focus_carrier_tool AS carrier
        ON carrier.tenant_id = focus.tenant_id
       AND carrier.person_email = focus.person_email
       AND carrier.activity_date = focus.activity_date
),
-- A tool reports whichever modalities it covers, so each branch carries its own
-- facts and leaves the rest unknown; the fold below sums past the unknowns.
person_day_tool_contributions AS (
    SELECT
        tenant_id,
        person_email,
        activity_date,
        tool,
        total_chat_messages,
        channel_posts_total,
        direct_and_group_messages,
        CAST(NULL AS Nullable(Float64)) AS emails_sent,
        CAST(NULL AS Nullable(Float64)) AS emails_received,
        CAST(NULL AS Nullable(Float64)) AS emails_read,
        CAST(NULL AS Nullable(Float64)) AS files_engaged,
        CAST(NULL AS Nullable(Float64)) AS files_shared_internal,
        CAST(NULL AS Nullable(Float64)) AS files_shared_external,
        CAST(NULL AS Nullable(Int64)) AS meeting_seconds,
        CAST(NULL AS Nullable(Float64)) AS meeting_hours,
        CAST(NULL AS Nullable(Int64)) AS meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS meetings_organized,
        CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_attended,
        CAST(NULL AS Nullable(Float64)) AS focus_hours,
        CAST(NULL AS Nullable(Float64)) AS working_hours,
        is_chat_active,
        toUInt8(0) AS is_email_active,
        toUInt8(0) AS is_documents_active,
        toUInt8(0) AS is_meetings_active,
        is_chat_active AS is_deliberately_active
    FROM chat_activity

    UNION ALL

    SELECT
        tenant_id,
        person_email,
        activity_date,
        tool,
        CAST(NULL AS Nullable(Int64)) AS total_chat_messages,
        CAST(NULL AS Nullable(Int64)) AS channel_posts_total,
        CAST(NULL AS Nullable(Int64)) AS direct_and_group_messages,
        emails_sent,
        emails_received,
        emails_read,
        CAST(NULL AS Nullable(Float64)) AS files_engaged,
        CAST(NULL AS Nullable(Float64)) AS files_shared_internal,
        CAST(NULL AS Nullable(Float64)) AS files_shared_external,
        CAST(NULL AS Nullable(Int64)) AS meeting_seconds,
        CAST(NULL AS Nullable(Float64)) AS meeting_hours,
        CAST(NULL AS Nullable(Int64)) AS meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS meetings_organized,
        CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_attended,
        CAST(NULL AS Nullable(Float64)) AS focus_hours,
        CAST(NULL AS Nullable(Float64)) AS working_hours,
        toUInt8(0) AS is_chat_active,
        is_email_active,
        toUInt8(0) AS is_documents_active,
        toUInt8(0) AS is_meetings_active,
        is_email_active AS is_deliberately_active
    FROM email_activity

    UNION ALL

    SELECT
        tenant_id,
        person_email,
        activity_date,
        tool,
        CAST(NULL AS Nullable(Int64)) AS total_chat_messages,
        CAST(NULL AS Nullable(Int64)) AS channel_posts_total,
        CAST(NULL AS Nullable(Int64)) AS direct_and_group_messages,
        CAST(NULL AS Nullable(Float64)) AS emails_sent,
        CAST(NULL AS Nullable(Float64)) AS emails_received,
        CAST(NULL AS Nullable(Float64)) AS emails_read,
        files_engaged,
        files_shared_internal,
        files_shared_external,
        CAST(NULL AS Nullable(Int64)) AS meeting_seconds,
        CAST(NULL AS Nullable(Float64)) AS meeting_hours,
        CAST(NULL AS Nullable(Int64)) AS meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS meetings_organized,
        CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_attended,
        CAST(NULL AS Nullable(Float64)) AS focus_hours,
        CAST(NULL AS Nullable(Float64)) AS working_hours,
        toUInt8(0) AS is_chat_active,
        toUInt8(0) AS is_email_active,
        is_documents_active,
        toUInt8(0) AS is_meetings_active,
        is_documents_active AS is_deliberately_active
    FROM document_activity

    UNION ALL

    SELECT
        tenant_id,
        person_email,
        activity_date,
        tool,
        CAST(NULL AS Nullable(Int64)) AS total_chat_messages,
        CAST(NULL AS Nullable(Int64)) AS channel_posts_total,
        CAST(NULL AS Nullable(Int64)) AS direct_and_group_messages,
        CAST(NULL AS Nullable(Float64)) AS emails_sent,
        CAST(NULL AS Nullable(Float64)) AS emails_received,
        CAST(NULL AS Nullable(Float64)) AS emails_read,
        CAST(NULL AS Nullable(Float64)) AS files_engaged,
        CAST(NULL AS Nullable(Float64)) AS files_shared_internal,
        CAST(NULL AS Nullable(Float64)) AS files_shared_external,
        meeting_seconds,
        meeting_hours,
        meetings_attended,
        meetings_organized,
        adhoc_meetings_attended,
        scheduled_meetings_attended,
        CAST(NULL AS Nullable(Float64)) AS focus_hours,
        CAST(NULL AS Nullable(Float64)) AS working_hours,
        toUInt8(0) AS is_chat_active,
        toUInt8(0) AS is_email_active,
        toUInt8(0) AS is_documents_active,
        is_meetings_active,
        is_meetings_active AS is_deliberately_active
    FROM meeting_activity

    UNION ALL

    SELECT
        tenant_id,
        person_email,
        activity_date,
        tool,
        CAST(NULL AS Nullable(Int64)) AS total_chat_messages,
        CAST(NULL AS Nullable(Int64)) AS channel_posts_total,
        CAST(NULL AS Nullable(Int64)) AS direct_and_group_messages,
        CAST(NULL AS Nullable(Float64)) AS emails_sent,
        CAST(NULL AS Nullable(Float64)) AS emails_received,
        CAST(NULL AS Nullable(Float64)) AS emails_read,
        CAST(NULL AS Nullable(Float64)) AS files_engaged,
        CAST(NULL AS Nullable(Float64)) AS files_shared_internal,
        CAST(NULL AS Nullable(Float64)) AS files_shared_external,
        CAST(NULL AS Nullable(Int64)) AS meeting_seconds,
        CAST(NULL AS Nullable(Float64)) AS meeting_hours,
        CAST(NULL AS Nullable(Int64)) AS meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS meetings_organized,
        CAST(NULL AS Nullable(Int64)) AS adhoc_meetings_attended,
        CAST(NULL AS Nullable(Int64)) AS scheduled_meetings_attended,
        focus_hours,
        working_hours,
        toUInt8(0) AS is_chat_active,
        toUInt8(0) AS is_email_active,
        toUInt8(0) AS is_documents_active,
        toUInt8(0) AS is_meetings_active,
        toUInt8(0) AS is_deliberately_active
    FROM focus_activity
)

SELECT
    -- SAFETY: every contributing branch admits only non-null keys.
    assumeNotNull(tenant_id) AS tenant_id,
    assumeNotNull(person_email) AS person_email,
    assumeNotNull(activity_date) AS activity_date,
    tool AS tool,
    {{ collab_tool_label('tool') }} AS tool_label,
    sum(total_chat_messages) AS total_chat_messages,
    sum(channel_posts_total) AS channel_posts_total,
    sum(direct_and_group_messages) AS direct_and_group_messages,
    sum(emails_sent) AS emails_sent,
    sum(emails_received) AS emails_received,
    sum(emails_read) AS emails_read,
    sum(files_engaged) AS files_engaged,
    sum(files_shared_internal) AS files_shared_internal,
    sum(files_shared_external) AS files_shared_external,
    sum(meeting_seconds) AS meeting_seconds,
    sum(meeting_hours) AS meeting_hours,
    sum(meetings_attended) AS meetings_attended,
    sum(meetings_organized) AS meetings_organized,
    sum(adhoc_meetings_attended) AS adhoc_meetings_attended,
    sum(scheduled_meetings_attended) AS scheduled_meetings_attended,
    sum(focus_hours) AS focus_hours,
    sum(working_hours) AS working_hours,
    max(is_chat_active) AS is_chat_active,
    max(is_email_active) AS is_email_active,
    max(is_documents_active) AS is_documents_active,
    max(is_meetings_active) AS is_meetings_active,
    max(is_deliberately_active) AS is_deliberately_active
FROM person_day_tool_contributions
GROUP BY tenant_id, person_email, activity_date, tool
