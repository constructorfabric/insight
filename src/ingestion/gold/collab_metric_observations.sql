{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['source_key', 'measure_key', 'entity_id', 'metric_date'],
    schema='insight',
    alias='collab_metric_observations',
    tags=['gold']
) }}

-- Source measure observations for the unified metrics runtime, collaboration
-- family. Reads class contracts only; no vendor-specific columns or tool
-- names appear inline. Every measure is emitted through the shape macros in
-- macros/metric_observation_measures.sql; the `tool` dimension display label
-- comes from macros/collab_tool_label.sql (static product vocabulary,
-- computed here rather than in silver so label changes apply retroactively on
-- the next build).
--
-- Materialized as a sorted table: the observation pipeline below (FINAL
-- dedup, nineteen measure branches) runs once per dbt build — which is
-- also the only time the silver inputs can have changed — instead of once
-- per metric query. The ordering key mirrors the runtime's filter shape
-- (source_key, measure_key, entity_id, metric_date), so single-measure
-- queries read index-pruned ranges rather than the whole relation.
--
-- The `tool` dimension value is the `data_source` discriminator with the
-- `insight_` prefix stripped (m365, slack, zoom, zulip_proxy). All Microsoft
-- 365 surfaces (Teams, Outlook, OneDrive, SharePoint) share the m365 tool.
--
-- Grain per measure:
--   day-grain sums (per tool):   total_chat_messages, channel_posts,
--                    direct_and_group_messages, emails_sent/received/read,
--                    files_engaged, files_shared_internal/external,
--                    meeting_hours, meetings_attended/organized,
--                    adhoc/scheduled_meetings_attended
--   day-grain sums (per scope):  files_shared (recipient scope
--                    internal/external — the combined files-shared total
--                    with a scope breakdown)
--   day-grain presence (per tool): chat_active_day (days with chat
--                    messages — the messages-per-active-day denominator,
--                    matching the day set its numerator draws from)
--   day-grain sums (no tool):    focus_hours, working_hours (HR-derived);
--                    meeting_free_day (0/1 on every deliberately-active
--                    day — a measured 0 for people in meetings daily,
--                    never conflated with missing coverage)
--   distinct-count subjects:     active_day (subject = date, per tool),
--                    active_modality (subject = modality)
--
-- Distinct-count measures carry a `subject_key`; the other macros do not, so
-- they are unioned in a separate branch that stamps subject_key = NULL on the
-- value/presence measures at the final projection.
--
-- Attribution: every measure keys on the class `person_key`, and only
-- email-shaped keys pass. The class contract falls back to lower(user_id)
-- where a source has no email (Slack), but cohorts and API requests address
-- people by email — a user-id key can never match either, so those rows are
-- excluded as unmatchable rather than carried as dead entities.
--
-- `focus_hours` / `working_hours` come from class_focus_metrics, which joins
-- HR scheduled hours. focus_time_pct is their ratio (× 100). Working hours
-- default to a nominal eight-hour day where the HR source omits them, and
-- focus is defined only on days a person has meeting records.
--
-- Memory shape (the measure branches run as concurrent pipelines within
-- the build query): every read keeps FINAL — the cheapest dedup, a
-- streaming merge of sorted parts — because no branch is duplicate-immune:
-- sum and presence measures inflate on duplicate row versions, and the
-- deliberate-activity gates (`> 0`) would pass a stale version's value.
-- The class tables are person x day x source grain (no per-event or
-- per-file rows), and the model contains no joins — the HR join lives in
-- silver (class_focus_metrics, materialized) — so no wide rows or strings
-- cross a join boundary.
--
-- Peer measurability (who enters a metric's peer pool) is decided HERE, by
-- row emission — the runtime never fabricates a zero for an entity with no
-- rows (see metrics DESIGN, "Peer measurability"). Each measure's emission
-- gate is therefore a deliberate semantic choice:
--   * value-gated (rows whenever the source reports the person, zeros
--     included): all volume counts (messages, emails, files, meetings).
--     A reported zero is a real behavioral observation — a quiet email
--     week — and belongs in peer pools.
--   * engagement-gated (rows only on deliberate activity): active_day,
--     active_modality, chat_active_day, and meeting_free_day (0/1 per
--     active day — "worked uninterrupted" is only defined on days the
--     person worked). A zero here would mean non-engagement (rostered
--     accounts with no activity: leavers, leave, service accounts), which
--     would drag peer medians toward zero and rank absent people; pools
--     compare engaged users among engaged users, matching the ai/git
--     activity metrics on the same dashboard.
-- Changing a gate re-ranks every peer standing for that metric — make it
-- an explicit decision, never a side effect of a connector reshaping its
-- emission.

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
        -- Recipient-scope dimension for the combined files_shared measure
        -- (static product vocabulary, like the tool labels).
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
-- A day/tool is active on a deliberate signal only (a message or email sent,
-- a file engaged or shared, a meeting attended); passive email received/read
-- is excluded. Deduped to one row per (tenant, entity, date, tool) and
-- re-labelled with the canonical suite label: the runtime groups breakdowns
-- by (value, label), so carrying the per-surface labels (Teams vs
-- Microsoft 365) across this union would split one m365 platform into two
-- active_day groups.
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
-- A meeting-free day: a DELIBERATELY-ACTIVE day (any modality) with zero
-- meeting time across every meeting tool. Anchoring on activity rather than
-- meeting records keeps the metric defined for people whose meeting tool
-- only writes rows on attendance (Zoom), and reads as "worked uninterrupted"
-- rather than "had an empty meeting report". Emitted as a 0/1 value on every
-- active day: a person in meetings every working day is a measured 0 — in
-- peer pools, scored — never conflated with a person who wasn't active at
-- all. Join-free: active-day markers and meeting seconds union into one
-- day-grain aggregate.
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

    -- Posts + thread replies: Slack folds replies into channel_posts (it
    -- cannot split them; channel_replies is NULL), M365 reports them apart —
    -- adding both keeps the count comparable across tools. NULL channel_posts
    -- (Zulip) stays NULL: the addition never resurrects an absent source.
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

    -- Combined files-shared total with a recipient-scope breakdown: the same
    -- measure emitted once per scope (mirrors ai's cost_usd emitted from two
    -- relations). Unfiltered queries sum both scopes; a scope breakdown
    -- splits them.
    {{ sum_measure('files_shared', 'document_source', 'shared_internally_count', 'internal_scope_dimensions') }}

    UNION ALL

    {{ sum_measure('files_shared', 'document_source', 'shared_externally_count', 'external_scope_dimensions') }}

    UNION ALL

    -- ifNull per modality, not a bare greatest(): Zoom rows carry NULL for
    -- modalities it does not report, and greatest() over a NULL argument is
    -- version-dependent in ClickHouse (NULL before 24.12, ignored after) —
    -- a bare form silently drops those rows' real audio time on older
    -- servers. Mirrors the modality handling the silver focus model uses.
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

    -- One row of 1 per (entity, date, tool) with chat messages: the
    -- messages-per-active-day denominator. Deliberately chat-gated, not the
    -- all-modality active_day — the ratio's numerator is chat messages, so
    -- its denominator must count only chat-active days.
    {{ sum_measure('chat_active_day', 'chat_source', 'if(total_chat_messages > 0, 1, NULL)', 'tool_dimensions') }}

    UNION ALL

    {{ sum_measure('meeting_free_day', 'meeting_free_source', 'meeting_free_flag', 'no_dimensions') }}
),
-- distinct_measure emits one row per source row, so each emission dedups
-- deliberate_activity (tool x modality grain) to its own grain first:
-- active_day to (entity, date, tool), active_modality to (entity, date,
-- modality) — otherwise a chat+email day duplicates active_day rows and a
-- two-tool modality duplicates active_modality rows, violating the
-- published unique-grain contract.
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
    value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
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
    value,
    subject_key,
    dimensions
FROM subject_measures
WHERE tenant_id IS NOT NULL
  AND entity_id IS NOT NULL
  AND metric_date IS NOT NULL
