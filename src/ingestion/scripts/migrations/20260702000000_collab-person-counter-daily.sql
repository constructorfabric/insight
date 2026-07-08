-- Gold view: insight.collab_person_counter_daily  (issue #1527, epic #1516)
--
-- The SHARED SCAFFOLD for the modality-named collaboration counters. This is
-- the first modality (Messaging) and lays the skeleton the other five
-- (#1528–#1532: meetings, email, documents, knowledge base, cadence) extend by
-- adding columns + UNION-ALL source branches — NOT new vendor-named rows.
--
-- Shape (clone of #1514's `ai_person_counter_daily`):
--   • honest-NULL `UNION ALL` over the per-vendor silver chat classes, each
--     branch filling only the columns it can source and NULLing the rest;
--   • outer GROUP BY (person_id, metric_date) with the
--     `if(countIf(x IS NOT NULL) > 0, sumIf(x, …), NULL)` wrapper so a person
--     with NO source on a day yields NULL (honest), never a fake 0. A real 0
--     (a row that exists with a zero count) is preserved as 0;
--   • `LEFT JOIN insight.people` to carry `org_unit_id` for the peer bands
--     computed in the query_ref (#1527 PR B), keyed on `person_id = lower(email)`.
--
-- Metrics (Messaging modality):
--   messages_sent  Σ total_chat_messages across M365 Teams · Slack · Zulip.
--                  Vendor semantics differ (Slack is a superset incl. replies;
--                  M365 excludes group chats & replies) — treat as chat
--                  engagement, not a comparable absolute (see silver
--                  collaboration/schema.yml).
--   channel_posts  Σ (channel_posts + channel_replies) across M365 · Slack.
--                  Slack folds posts+replies into `channel_posts` already
--                  (`channel_replies` NULL); M365 splits them, so we add both
--                  for vendor comparability. Zulip does not surface channel
--                  posts → NULL (honest-NULL, not 0).
--
-- Source: silver.class_collab_chat_activity (grain: person, date, data_source).

DROP VIEW IF EXISTS insight.collab_person_counter_daily;
CREATE VIEW insight.collab_person_counter_daily AS
SELECT
    d.person_id AS person_id,
    p.org_unit_id AS org_unit_id,
    d.metric_date AS metric_date,
    d.messages_sent AS messages_sent,
    d.channel_posts AS channel_posts
FROM (
    SELECT
        person_id,
        metric_date,
        if(countIf(messages_sent IS NOT NULL) > 0, sumIf(messages_sent, messages_sent IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS messages_sent,
        if(countIf(channel_posts IS NOT NULL) > 0, sumIf(channel_posts, channel_posts IS NOT NULL), CAST(NULL AS Nullable(Float64))) AS channel_posts
    FROM (
        -- ─── M365 Teams ─────────────────────────────────────────────────
        -- channel_posts = postMessages + replyMessages (comparable to Slack,
        -- which folds replies into channel_posts).
        SELECT
            lower(c.email) AS person_id,
            c.date AS metric_date,
            if(c.total_chat_messages IS NULL, CAST(NULL AS Nullable(Float64)), toFloat64(c.total_chat_messages)) AS messages_sent,
            if(c.channel_posts IS NULL, CAST(NULL AS Nullable(Float64)), toFloat64(c.channel_posts) + toFloat64(ifNull(c.channel_replies, 0))) AS channel_posts
        -- FINAL dedups the ReplacingMergeTree at read time (a re-synced day can
        -- leave >1 version per key before a background merge) — matches the
        -- class_collab_meeting_activity read in 20260518000000_collab-bullet-rewrite.sql.
        FROM silver.class_collab_chat_activity AS c FINAL
        WHERE c.data_source = 'insight_m365'
          AND c.email IS NOT NULL
          AND c.email != ''

        UNION ALL

        -- ─── Slack ──────────────────────────────────────────────────────
        -- channel_posts already includes replies (Slack cannot split them, so
        -- channel_replies is NULL) — use it as-is.
        SELECT
            lower(s.email) AS person_id,
            s.date AS metric_date,
            if(s.total_chat_messages IS NULL, CAST(NULL AS Nullable(Float64)), toFloat64(s.total_chat_messages)) AS messages_sent,
            if(s.channel_posts IS NULL, CAST(NULL AS Nullable(Float64)), toFloat64(s.channel_posts)) AS channel_posts
        FROM silver.class_collab_chat_activity AS s FINAL
        WHERE s.data_source = 'insight_slack'
          AND s.email IS NOT NULL
          AND s.email != ''

        UNION ALL

        -- ─── Zulip ──────────────────────────────────────────────────────
        -- Zulip proxy exposes only total chat messages; no channel-post split
        -- → channel_posts is honest-NULL.
        SELECT
            lower(z.email) AS person_id,
            z.date AS metric_date,
            if(z.total_chat_messages IS NULL, CAST(NULL AS Nullable(Float64)), toFloat64(z.total_chat_messages)) AS messages_sent,
            CAST(NULL AS Nullable(Float64)) AS channel_posts
        FROM silver.class_collab_chat_activity AS z FINAL
        WHERE z.data_source = 'insight_zulip_proxy'
          AND z.email IS NOT NULL
          AND z.email != ''
    ) raw
    GROUP BY person_id, metric_date
) d
LEFT JOIN insight.people AS p ON d.person_id = p.person_id;
