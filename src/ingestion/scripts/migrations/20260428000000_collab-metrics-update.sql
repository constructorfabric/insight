-- =====================================================================
-- Collaboration metrics update — rename mislabeled keys, split mixed
-- aggregations, add consistency metrics
-- =====================================================================
--
-- Audit pass over the Slack and Microsoft 365 (Outlook / Teams /
-- OneDrive / SharePoint) bullet metrics. Two of the previously shipped
-- keys misrepresented their underlying bronze data; this migration
-- renames them to match what the source actually provides, splits one
-- mixed metric into the parts that drive different decisions, and adds
-- consistency / engagement metrics built from columns that were already
-- in silver but not surfaced.
--
-- Slack renames
--   slack_message_engagement   → slack_messages_sent
--     Was total_chat_messages labelled "Message Engagement (avg replies
--     per thread)". Slack analytics endpoint does not split posts vs
--     replies — renamed to plain "Messages Sent".
--   slack_thread_participation → slack_channel_posts
--     Was channel_messages_posted_count labelled "Thread Participation
--     (replies to others' threads)". Bronze does not separate post vs
--     reply — renamed to "Channel Posts".
--
-- New Slack metrics (both from total_chat_messages)
--   slack_active_days          1/0 per day → period total = active days
--   slack_msgs_per_active_day  msgs on active days, NULL otherwise
--                              (CH avg ignores NULL → mean intensity
--                              over active days)
--
-- Microsoft 365 rename
--   m365_teams_messages → m365_teams_chats
--     Was total_chat_messages = direct + group, but labelled "Teams
--     Messages · all channels sent". Teams analytics does not expose
--     channel-post granularity here — renamed to "Teams Chats" to
--     reflect what the data actually contains (DMs + group chats).
--
-- Microsoft 365 split
--   m365_files_shared → m365_files_shared_internal +
--                       m365_files_shared_external
--     Internal sharing is a collaboration signal; external sharing is a
--     governance / DLP signal. Combined into one metric they obscure
--     each other.
--
-- New Microsoft 365 metrics (data already in silver, not surfaced)
--   m365_emails_received   received_count (sum). Inbox volume — needed
--                          context for the existing m365_emails_sent.
--   m365_emails_read       read_count (sum). Read-activity volume.
--                          We do NOT compute read/received ratio because
--                          the two columns are not time-aligned — read
--                          can include emails received earlier, so the
--                          daily ratio routinely exceeds 100% and
--                          becomes meaningless after avg-per-person.
--   m365_files_engaged     viewed_or_edited_count (sum). Real document
--                          collaboration activity, not just sharing.
--   m365_active_days       1/0 per day across email + Teams + docs
--                          surfaces. Generic "did anything happen on
--                          M365 that day".
--
-- Meetings rename + source switch
--   zoom_calls → zoom_meetings
--     calls_count is NULL by design in the Zoom dbt model (zoom__collab_
--     meeting_activity.sql sets it to NULL — Zoom analytics does not
--     expose a calls_count distinct from meetings). The old metric was
--     therefore always 0. Switched to meetings_attended (count of
--     distinct Zoom meetings the user joined that day).
--
-- Meeting metrics — drop dependency on silver.class_focus_metrics
--   meeting_hours and meeting_free now read from
--   silver.class_collab_meeting_activity FINAL directly. focus_metrics
--   (silver/hr/) is plain MergeTree — incremental upserts left ×N stale
--   data after silver dedup of class_collab_meeting_activity in #237.
--   Reading silver primitive directly bypasses the issue. Cap
--   `least(meeting_hours, working_hours_per_day)` removed — silver is
--   now correct, no need to mask "12h meetings" down to 8h. Pipeline
--   fix for focus_metrics tracked separately.
--
-- Supersedes the slack/m365/meetings branches of insight.collab_bullet_
-- rows defined in 20260427120000_views-from-silver.sql. View is
-- DROP+CREATE so re-running this migration is idempotent.
-- =====================================================================

DROP VIEW IF EXISTS insight.collab_bullet_rows;
CREATE VIEW insight.collab_bullet_rows AS
SELECT
    lower(e.email)                                AS person_id,
    p.org_unit_id,
    toString(e.date)                              AS metric_date,
    'm365_emails_sent'                            AS metric_key,
    toFloat64(ifNull(e.sent_count, 0))            AS metric_value
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
WHERE e.data_source = 'insight_m365'
  AND e.email IS NOT NULL
  AND e.email != ''

UNION ALL
SELECT
    lower(e.email), p.org_unit_id, toString(e.date), 'm365_emails_received',
    toFloat64(ifNull(e.received_count, 0))
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
WHERE e.data_source = 'insight_m365'
  AND e.email IS NOT NULL
  AND e.email != ''

UNION ALL
SELECT
    lower(e.email), p.org_unit_id, toString(e.date), 'm365_emails_read',
    toFloat64(ifNull(e.read_count, 0))
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
WHERE e.data_source = 'insight_m365'
  AND e.email IS NOT NULL
  AND e.email != ''

-- Meeting metrics: read from silver.class_collab_meeting_activity FINAL
-- directly (deduped via ReplacingMergeTree). Avoids silver.class_focus_metrics
-- which is plain MergeTree with stale ×N values until the data team fixes
-- engine + full-refresh (separate issue, scope of silver/hr/).
--
-- Split by source: M365 (Teams) and Zoom go in different bullets so the
-- distribution stays meaningful. People who only use one platform should
-- not be averaged with people who use both. Old `zoom_calls` was always
-- 0 (calls_count NULL by design in zoom dbt) — replaced by per-source
-- meetings_count + meeting_hours pairs. No working_hours cap — silver is
-- correct, real "12h of meetings" surfaces as 12h.

-- meeting_hours: total across both sources (M365 + Zoom)
UNION ALL
SELECT
    lower(ma.email), p.org_unit_id, toString(ma.date), 'meeting_hours',
    sum(greatest(
        ifNull(ma.audio_duration_seconds, 0),
        ifNull(ma.video_duration_seconds, 0),
        ifNull(ma.screen_share_duration_seconds, 0)
    )) / 3600.0
FROM silver.class_collab_meeting_activity AS ma FINAL
LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
WHERE ma.email IS NOT NULL AND ma.email != ''
GROUP BY lower(ma.email), p.org_unit_id, ma.date

-- meetings_count: total distinct meetings attended across both sources
UNION ALL
SELECT
    lower(ma.email), p.org_unit_id, toString(ma.date), 'meetings_count',
    sum(toFloat64(ifNull(ma.meetings_attended, 0)))
FROM silver.class_collab_meeting_activity AS ma FINAL
LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
WHERE ma.email IS NOT NULL AND ma.email != ''
GROUP BY lower(ma.email), p.org_unit_id, ma.date

-- teams_meeting_hours: longest modality per row from M365 source (Teams)
UNION ALL
SELECT
    lower(ma.email), p.org_unit_id, toString(ma.date), 'teams_meeting_hours',
    sum(greatest(
        ifNull(ma.audio_duration_seconds, 0),
        ifNull(ma.video_duration_seconds, 0),
        ifNull(ma.screen_share_duration_seconds, 0)
    )) / 3600.0
FROM silver.class_collab_meeting_activity AS ma FINAL
LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
WHERE ma.data_source = 'insight_m365'
  AND ma.email IS NOT NULL
  AND ma.email != ''
GROUP BY lower(ma.email), p.org_unit_id, ma.date

-- zoom_meeting_hours: same formula, Zoom source only
UNION ALL
SELECT
    lower(ma.email), p.org_unit_id, toString(ma.date), 'zoom_meeting_hours',
    sum(greatest(
        ifNull(ma.audio_duration_seconds, 0),
        ifNull(ma.video_duration_seconds, 0),
        ifNull(ma.screen_share_duration_seconds, 0)
    )) / 3600.0
FROM silver.class_collab_meeting_activity AS ma FINAL
LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
WHERE ma.data_source = 'insight_zoom'
  AND ma.email IS NOT NULL
  AND ma.email != ''
GROUP BY lower(ma.email), p.org_unit_id, ma.date

-- teams_meetings: count of distinct M365 (Teams) meetings attended
UNION ALL
SELECT
    lower(ma.email), p.org_unit_id, toString(ma.date), 'teams_meetings',
    toFloat64(ifNull(ma.meetings_attended, 0))
FROM silver.class_collab_meeting_activity AS ma FINAL
LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
WHERE ma.data_source = 'insight_m365'
  AND ma.email IS NOT NULL
  AND ma.email != ''

-- zoom_meetings: count of distinct Zoom meetings joined
UNION ALL
SELECT
    lower(ma.email), p.org_unit_id, toString(ma.date), 'zoom_meetings',
    toFloat64(ifNull(ma.meetings_attended, 0))
FROM silver.class_collab_meeting_activity AS ma FINAL
LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
WHERE ma.data_source = 'insight_zoom'
  AND ma.email IS NOT NULL
  AND ma.email != ''

UNION ALL
SELECT
    lower(c.email), p.org_unit_id, toString(c.date), 'm365_teams_chats',
    toFloat64(c.total_chat_messages)
FROM silver.class_collab_chat_activity AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.data_source = 'insight_m365'
  AND c.email IS NOT NULL
  AND c.email != ''

UNION ALL
SELECT
    lower(d.email), p.org_unit_id, toString(d.date), 'm365_files_shared_internal',
    toFloat64(ifNull(d.shared_internally_count, 0))
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
WHERE d.data_source = 'insight_m365'
  AND d.email IS NOT NULL
  AND d.email != ''

UNION ALL
SELECT
    lower(d.email), p.org_unit_id, toString(d.date), 'm365_files_shared_external',
    toFloat64(ifNull(d.shared_externally_count, 0))
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
WHERE d.data_source = 'insight_m365'
  AND d.email IS NOT NULL
  AND d.email != ''

UNION ALL
SELECT
    lower(d.email), p.org_unit_id, toString(d.date), 'm365_files_engaged',
    toFloat64(ifNull(d.viewed_or_edited_count, 0))
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
WHERE d.data_source = 'insight_m365'
  AND d.email IS NOT NULL
  AND d.email != ''

-- m365_active_days: any DELIBERATE activity that day across email,
-- Teams chat, or documents. Counts only actions the user explicitly
-- took: sent_count (not received_count — inbox arrivals are passive),
-- chat messages posted, file edits / shares. Aggregation = sum →
-- period total of active days.
UNION ALL
SELECT
    person_id, any(p.org_unit_id) AS org_unit_id, metric_date, 'm365_active_days',
    if(sum(activity) > 0, toFloat64(1), toFloat64(0))
FROM (
    SELECT
        lower(email)                                  AS person_id,
        toString(date)                                AS metric_date,
        toFloat64(ifNull(sent_count, 0))              AS activity
    FROM silver.class_collab_email_activity
    WHERE data_source = 'insight_m365'
        AND email IS NOT NULL
        AND email != ''

    UNION ALL
    SELECT
        lower(email), toString(date),
        toFloat64(ifNull(total_chat_messages, 0))
    FROM silver.class_collab_chat_activity
    WHERE data_source = 'insight_m365'
        AND email IS NOT NULL
        AND email != ''

    UNION ALL
    SELECT
        lower(email), toString(date),
        toFloat64(ifNull(viewed_or_edited_count, 0)) +
        toFloat64(ifNull(shared_internally_count, 0)) +
        toFloat64(ifNull(shared_externally_count, 0))
    FROM silver.class_collab_document_activity
    WHERE data_source = 'insight_m365'
        AND email IS NOT NULL
        AND email != ''
) AS m365_daily
LEFT JOIN insight.people AS p ON p.person_id = m365_daily.person_id
GROUP BY person_id, metric_date

-- meeting_free: 1 per (person, day) where summed meeting durations = 0.
-- Same source as meeting_hours so coverage is consistent. Aggregation = sum
-- → period total = days with any record but no actual meeting time.
UNION ALL
SELECT
    lower(ma.email), p.org_unit_id, toString(ma.date), 'meeting_free',
    if(sum(
        ifNull(ma.audio_duration_seconds, 0) +
        ifNull(ma.video_duration_seconds, 0) +
        ifNull(ma.screen_share_duration_seconds, 0)
    ) = 0, toFloat64(1), toFloat64(0))
FROM silver.class_collab_meeting_activity AS ma FINAL
LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
WHERE ma.email IS NOT NULL AND ma.email != ''
GROUP BY lower(ma.email), p.org_unit_id, ma.date

-- Slack ----------------------------------------------------------------
UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_messages_sent',
    toFloat64(ifNull(s.total_chat_messages, 0))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack'
  AND s.email IS NOT NULL
  AND s.email != ''

UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_channel_posts',
    toFloat64(ifNull(s.channel_posts, 0))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack'
  AND s.email IS NOT NULL
  AND s.email != ''

UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_active_days',
    if(ifNull(s.total_chat_messages, 0) > 0, toFloat64(1), toFloat64(0))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack'
  AND s.email IS NOT NULL
  AND s.email != ''

UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_msgs_per_active_day',
    if(ifNull(s.total_chat_messages, 0) > 0,
       toFloat64(s.total_chat_messages),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack'
  AND s.email IS NOT NULL
  AND s.email != ''

UNION ALL
SELECT
    lower(s.email), p.org_unit_id, toString(s.date), 'slack_dm_ratio',
    if(ifNull(s.total_chat_messages, 0) > 0,
       round(((toFloat64(ifNull(s.total_chat_messages, 0)) -
               toFloat64(ifNull(s.channel_posts, 0))) /
              toFloat64(s.total_chat_messages)) * 100, 1),
       CAST(NULL AS Nullable(Float64)))
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
WHERE s.data_source = 'insight_slack'
  AND s.email IS NOT NULL
  AND s.email != '';
