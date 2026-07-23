-- depends_on: {{ ref('m365__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    on_schema_change='append_new_columns',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['m365', 'silver:class_collab_chat_activity']
) }}

-- Chat-message column semantics — see issues #431 and #266.
--
-- `getTeamsUserActivityUserDetail` exposes the following counters:
--   • privateChatMessageCount → 1:1 DMs            (correct → direct_messages)
--   • teamChatMessageCount    → channel posts      (per Microsoft Graph docs:
--                                                  "messages posted in a Teams
--                                                  channel, excluding replies".
--                                                  Despite the name, this is
--                                                  NOT group DMs.)
--   • postMessages            → channel thread starts
--   • replyMessages           → channel replies
--   • urgentMessages          → urgent-flagged
--
-- The report endpoint does NOT expose group-chat (multi-party DM) counts at
-- all. Microsoft's only path is the content-bearing `/chats` API with
-- `Chat.Read.All` scope. Until/unless that stream lands, group-chat counts
-- are honestly NULL for m365 (#431).
--
-- `direct_and_group_messages` (#266 sibling): for m365 only the DM half is
-- available; group is unsurfaced. We emit `privateChatMessageCount` (1:1
-- DMs only) and document the gap in silver schema. Cross-vendor aggregates
-- that compare with Slack's `total - channel` residual must account for
-- this asymmetry.

SELECT
    tenant_id,
    source_id AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-', coalesce(userPrincipalName, ''), '-', toString(reportRefreshDate))) AS unique_key,
    userPrincipalName AS user_id,
    userPrincipalName AS user_name,
    userPrincipalName AS email,
    if(userPrincipalName IS NOT NULL AND userPrincipalName != '',
       lower(userPrincipalName),
       '') AS person_key,
    toDate(reportRefreshDate) AS date,
    privateChatMessageCount AS direct_messages,
    -- #431: teamChatMessageCount is channel-post activity (not group DMs)
    -- per Microsoft Graph docs. Group-chat counts are not surfaced by this
    -- report endpoint. Emit NULL rather than the mislabeled channel count.
    CAST(NULL AS Nullable(Int64)) AS group_chat_messages,
    -- #266: for m365, only the DM portion of "direct + group" is available.
    -- Group chats unsurfaced — see header.
    privateChatMessageCount AS direct_and_group_messages,
    -- total_chat_messages retains the existing semantics
    -- (DMs + team-channel messages) so existing Gold consumers do not see a
    -- discontinuity. This is "user engagement across DM + channel surfaces",
    -- not "DM + group DM". Documented in silver schema.
    COALESCE(privateChatMessageCount, 0) + COALESCE(teamChatMessageCount, 0) AS total_chat_messages,
    postMessages AS channel_posts,
    replyMessages AS channel_replies,
    urgentMessages AS urgent_messages,
    reportPeriod AS report_period,
    now() AS collected_at,
    'insight_m365' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM {{ source('bronze_m365', 'teams_activity') }}
WHERE userPrincipalName IS NOT NULL
  AND userPrincipalName != ''
  -- Drop unlicensed users (guests, ex-employees, service accounts). Their Teams
  -- activity inflates team-level collab counters and produces orphan gold rows
  -- with no matching insight.people entry. `isLicensed` = "Selected if the user
  -- is licensed to use Teams" (MS Graph getTeamsUserActivityUserDetail). Only the
  -- teams_activity report exposes this flag; the email/onedrive/sharepoint feeders
  -- fall back to `assignedProducts`. A NULL flag is treated as unlicensed and
  -- dropped too, per the #736 proposal. See #736.
  AND coalesce(isLicensed, false) = true
{% if is_incremental() %}
  -- Watermark on the source EXTRACT time, not the business date (see zoom model header
  -- for the backfill-strand failure mode this fixes). Re-pulled rows carry a fresh
  -- `_airbyte_extracted_at`, so reprocess every business date touched by a recent extract.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(reportRefreshDate) IN (
      SELECT DISTINCT toDate(reportRefreshDate)
      FROM {{ source('bronze_m365', 'teams_activity') }}
      WHERE _airbyte_extracted_at
            > (SELECT max(_airbyte_extracted_at) FROM {{ source('bronze_m365', 'teams_activity') }}) - INTERVAL 3 DAY
    )
  )
{% endif %}
