-- =====================================================================
-- collab_bullet_rows — Phase A rewrite (issue #433 §4.1)
-- =====================================================================
--
-- Same rewrite as `20260515000000_task-delivery-bullet-rewrite.sql`
-- applied to the collaboration section. Three concurrent changes:
--
--   1. SCAN CONSOLIDATION (issue #433 §3.5). View dropped from 19
--      UNION-ALL branches to 6 — one per source class. Within each
--      branch multiple metrics are emitted via `ARRAY JOIN` over a
--      tuple array, so each silver class is read once and the row is
--      unpacked into N rows (one per metric). The previous shape made
--      ClickHouse re-scan `class_collab_email_activity` three times,
--      `class_collab_meeting_activity` seven times, etc.
--
--   2. RATIO num/den SPLIT (issue #433 §3.3). The two daily-ratio
--      metrics were:
--
--        slack_msgs_per_active_day  daily total_chat_messages NULL-on-zero
--        slack_dm_ratio             daily 100*(total-channel)/total
--
--      Both went through `avg(metric_value)` in `query_ref` over the
--      period — mathematically wrong when daily denominators differ.
--      Both are dropped from this view as standalone `metric_key`s.
--      `query_ref` now reconstructs them as `Σnum / Σden` over the
--      period from the raw counters that are already emitted:
--
--        slack_msgs_per_active_day = Σslack_messages_sent
--                                  / Σslack_active_days
--        slack_dm_ratio            = 100
--                                  * (Σslack_messages_sent - Σslack_channel_posts)
--                                  / Σslack_messages_sent
--
--      The other ratio-shaped metric in this section, `meeting_free`,
--      is NOT a true ratio — it is a counter (1 per meeting-free day),
--      summed over the period. Keeps existing semantics.
--
--   3. `metric_date` type. Previously `String` via `toString(...)`;
--      now `Date` (the native type of each silver source column).
--      Unlocks MergeTree min/max statistics on any downstream
--      materialization. Existing range comparisons against ISO-8601
--      literals in `query_ref` work identically.
--
-- Branch shape after rewrite (6 branches, source-aligned):
--
--   1. `class_collab_email_activity`     → 3 keys via ARRAY JOIN
--   2. `class_collab_meeting_activity`   → 7 keys via wide-aggregate +
--                                          ARRAY JOIN (sumIf per source)
--   3. `class_collab_chat_activity` M365 → 1 key
--   4. `class_collab_chat_activity` Slack → 3 keys via ARRAY JOIN
--   4b. `class_collab_chat_activity` Zulip → 1 key (zulip_messages_sent)
--   5. `class_collab_document_activity`  → 3 keys via ARRAY JOIN
--   6. cross-class CTE                   → 1 key (m365_active_days)
--
-- 19 distinct metric_keys after the Zulip add (was 18). The two
-- composite-ratio keys (`slack_msgs_per_active_day`, `slack_dm_ratio`)
-- visible on the FE live ONLY in the `query_ref` projection — they are
-- not emitted by the view.
-- =====================================================================

DROP VIEW IF EXISTS insight.collab_bullet_rows;

CREATE VIEW insight.collab_bullet_rows AS

-- ─── Branch 1: class_collab_email_activity (M365 Outlook) ────────────
-- One silver row per (tenant, person, date) when data_source = 'insight_m365'.
-- ARRAY JOIN emits 3 metrics per row.
SELECT
    lower(e.email)                                  AS person_id,
    p.org_unit_id                                   AS org_unit_id,
    e.date                                          AS metric_date,
    kv.1                                            AS metric_key,
    kv.2                                            AS metric_value
FROM silver.class_collab_email_activity AS e
LEFT JOIN insight.people AS p ON lower(e.email) = p.person_id
ARRAY JOIN [
    ('m365_emails_sent',     toFloat64(ifNull(e.sent_count, 0))),
    ('m365_emails_received', toFloat64(ifNull(e.received_count, 0))),
    ('m365_emails_read',     toFloat64(ifNull(e.read_count, 0)))
] AS kv
WHERE e.data_source = 'insight_m365'
  AND e.email IS NOT NULL
  AND e.email != ''

UNION ALL

-- ─── Branch 2: class_collab_meeting_activity (M365 + Zoom) ───────────
-- Silver class has grain (tenant, person, date, data_source) so for a
-- given (person, date) there can be one row per source (Teams + Zoom).
-- Wide-aggregate the row pair via GROUP BY (person, date) using
-- `sum` for cross-source totals and `sumIf` for per-source aggregates.
-- Then ARRAY JOIN unpacks the 7 wide columns into 7 long rows.
SELECT
    pp.person_id                                    AS person_id,
    pp.org_unit_id                                  AS org_unit_id,
    pp.metric_date                                  AS metric_date,
    kv.1                                            AS metric_key,
    kv.2                                            AS metric_value
FROM (
    SELECT
        lower(ma.email)                             AS person_id,
        any(p.org_unit_id)                          AS org_unit_id,
        ma.date                                     AS metric_date,
        -- Cross-source: total meeting time (longest modality per row,
        -- summed across both sources for that day).
        sum(greatest(
            ifNull(ma.audio_duration_seconds, 0),
            ifNull(ma.video_duration_seconds, 0),
            ifNull(ma.screen_share_duration_seconds, 0)
        )) / 3600.0                                 AS meeting_hours_v,
        -- Cross-source: total meetings attended.
        sum(toFloat64(ifNull(ma.meetings_attended, 0)))
                                                    AS meetings_count_v,
        -- Per-source: Teams (M365).
        sumIf(greatest(
            ifNull(ma.audio_duration_seconds, 0),
            ifNull(ma.video_duration_seconds, 0),
            ifNull(ma.screen_share_duration_seconds, 0)
        ) / 3600.0,
              ma.data_source = 'insight_m365')      AS teams_meeting_hours_v,
        sumIf(greatest(
            ifNull(ma.audio_duration_seconds, 0),
            ifNull(ma.video_duration_seconds, 0),
            ifNull(ma.screen_share_duration_seconds, 0)
        ) / 3600.0,
              ma.data_source = 'insight_zoom')      AS zoom_meeting_hours_v,
        sumIf(toFloat64(ifNull(ma.meetings_attended, 0)),
              ma.data_source = 'insight_m365')      AS teams_meetings_v,
        sumIf(toFloat64(ifNull(ma.meetings_attended, 0)),
              ma.data_source = 'insight_zoom')      AS zoom_meetings_v,
        -- meeting_free: 1 if no meeting time on this day across either
        -- source, else 0. Period-aggregate via `sum` → count of
        -- meeting-free days.
        if(sum(ifNull(ma.audio_duration_seconds, 0)
             + ifNull(ma.video_duration_seconds, 0)
             + ifNull(ma.screen_share_duration_seconds, 0)) = 0,
           toFloat64(1),
           toFloat64(0))                            AS meeting_free_v
    FROM silver.class_collab_meeting_activity AS ma FINAL
    LEFT JOIN insight.people AS p ON lower(ma.email) = p.person_id
    WHERE ma.email IS NOT NULL AND ma.email != ''
    GROUP BY lower(ma.email), ma.date
) AS pp
ARRAY JOIN [
    ('meeting_hours',        pp.meeting_hours_v),
    ('meetings_count',       pp.meetings_count_v),
    ('teams_meeting_hours',  pp.teams_meeting_hours_v),
    ('zoom_meeting_hours',   pp.zoom_meeting_hours_v),
    ('teams_meetings',       pp.teams_meetings_v),
    ('zoom_meetings',        pp.zoom_meetings_v),
    ('meeting_free',         pp.meeting_free_v)
] AS kv

UNION ALL

-- ─── Branch 3: class_collab_chat_activity — M365 Teams ───────────────
-- Single key (m365_teams_chats), no ARRAY JOIN needed.
SELECT
    lower(c.email)                                  AS person_id,
    p.org_unit_id                                   AS org_unit_id,
    c.date                                          AS metric_date,
    'm365_teams_chats'                              AS metric_key,
    toFloat64(ifNull(c.total_chat_messages, 0))     AS metric_value
FROM silver.class_collab_chat_activity AS c
LEFT JOIN insight.people AS p ON lower(c.email) = p.person_id
WHERE c.data_source = 'insight_m365'
  AND c.email IS NOT NULL
  AND c.email != ''

UNION ALL

-- ─── Branch 4: class_collab_chat_activity — Slack ────────────────────
-- ARRAY JOIN emits 3 raw counters. The two composite ratios
-- (slack_msgs_per_active_day, slack_dm_ratio) are computed in query_ref
-- from these raw counters as Σnum/Σden.
SELECT
    lower(s.email)                                  AS person_id,
    p.org_unit_id                                   AS org_unit_id,
    s.date                                          AS metric_date,
    kv.1                                            AS metric_key,
    kv.2                                            AS metric_value
FROM silver.class_collab_chat_activity AS s
LEFT JOIN insight.people AS p ON lower(s.email) = p.person_id
ARRAY JOIN [
    ('slack_messages_sent', toFloat64(ifNull(s.total_chat_messages, 0))),
    ('slack_channel_posts', toFloat64(ifNull(s.channel_posts, 0))),
    ('slack_active_days',
        if(ifNull(s.total_chat_messages, 0) > 0,
           toFloat64(1),
           toFloat64(0)))
] AS kv
WHERE s.data_source = 'insight_slack'
  AND s.email IS NOT NULL
  AND s.email != ''

UNION ALL

-- ─── Branch 4b: class_collab_chat_activity — Zulip ───────────────────
-- Single key (zulip_messages_sent), no ARRAY JOIN needed. Mirrors the
-- m365 Teams branch: one silver row per (person, date) where
-- data_source = 'insight_zulip_proxy' → period-summed chat messages.
SELECT
    lower(z.email)                                  AS person_id,
    p.org_unit_id                                   AS org_unit_id,
    z.date                                          AS metric_date,
    'zulip_messages_sent'                           AS metric_key,
    toFloat64(ifNull(z.total_chat_messages, 0))     AS metric_value
FROM silver.class_collab_chat_activity AS z
LEFT JOIN insight.people AS p ON lower(z.email) = p.person_id
WHERE z.data_source = 'insight_zulip_proxy'
  AND z.email IS NOT NULL
  AND z.email != ''

UNION ALL

-- ─── Branch 5: class_collab_document_activity (M365 OneDrive/SharePoint) ─
-- ARRAY JOIN emits 3 raw counters.
SELECT
    lower(d.email)                                  AS person_id,
    p.org_unit_id                                   AS org_unit_id,
    d.date                                          AS metric_date,
    kv.1                                            AS metric_key,
    kv.2                                            AS metric_value
FROM silver.class_collab_document_activity AS d
LEFT JOIN insight.people AS p ON lower(d.email) = p.person_id
ARRAY JOIN [
    ('m365_files_shared_internal', toFloat64(ifNull(d.shared_internally_count, 0))),
    ('m365_files_shared_external', toFloat64(ifNull(d.shared_externally_count, 0))),
    ('m365_files_engaged',         toFloat64(ifNull(d.viewed_or_edited_count, 0)))
] AS kv
WHERE d.data_source = 'insight_m365'
  AND d.email IS NOT NULL
  AND d.email != ''

UNION ALL

-- ─── Branch 6: m365_active_days (cross-class CTE) ────────────────────
-- Any DELIBERATE M365 activity on a given day across email, Teams chat,
-- or documents. Counts only actions the user explicitly took:
-- sent_count (not received_count — inbox arrivals are passive), chat
-- messages posted, file edits / shares. Aggregation = sum → period
-- total of active days. Kept as a standalone branch because the CTE
-- spans three silver classes; ARRAY JOIN cannot consolidate across
-- different source tables.
SELECT
    person_id,
    any(p.org_unit_id)                              AS org_unit_id,
    metric_date,
    'm365_active_days'                              AS metric_key,
    if(sum(activity) > 0, toFloat64(1), toFloat64(0))
                                                    AS metric_value
FROM (
    SELECT
        lower(email)                                AS person_id,
        date                                        AS metric_date,
        toFloat64(ifNull(sent_count, 0))            AS activity
    FROM silver.class_collab_email_activity
    WHERE data_source = 'insight_m365'
      AND email IS NOT NULL
      AND email != ''

    UNION ALL
    SELECT
        lower(email), date,
        toFloat64(ifNull(total_chat_messages, 0))
    FROM silver.class_collab_chat_activity
    WHERE data_source = 'insight_m365'
      AND email IS NOT NULL
      AND email != ''

    UNION ALL
    SELECT
        lower(email), date,
        toFloat64(ifNull(viewed_or_edited_count, 0))
      + toFloat64(ifNull(shared_internally_count, 0))
      + toFloat64(ifNull(shared_externally_count, 0))
    FROM silver.class_collab_document_activity
    WHERE data_source = 'insight_m365'
      AND email IS NOT NULL
      AND email != ''
) AS m365_daily
LEFT JOIN insight.people AS p ON p.person_id = m365_daily.person_id
GROUP BY person_id, metric_date;
