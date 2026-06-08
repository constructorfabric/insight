-- depends_on: {{ ref('slack__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    on_schema_change='append_new_columns',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['slack', 'silver:class_collab_chat_activity']
) }}

-- Slack daily chat activity per user, sourced from admin.analytics.getFile?type=member.
-- Bronze row is already one per (user, date); we simply reshape to the shared
-- class_collab_chat_activity schema. The analytics endpoint reports
-- `messages_posted_count` (every message the user sent) and
-- `channel_messages_posted_count` (the channel slice) but does NOT break out
-- DMs from MPIMs from channel-thread replies.
--
-- direct_messages / group_chat_messages stay NULL — true DM/MPIM separation
-- needs the message-stream content connector + `*:history` scopes.
--
-- direct_and_group_messages (#266): the analytics endpoint's
-- `total - channel` residual is exactly "everything the user posted outside
-- channels" = DMs + MPIMs (and counts as the natural Slack mirror of
-- m365's `privateChatMessageCount` plus group chats — except Slack DOES
-- surface group chats inside this residual, where m365 does not). Cross-
-- vendor comparison caveats are documented in silver/collaboration/schema.yml.
-- `greatest(0, …)` guards against any Slack analytics off-by-one that would
-- otherwise produce a negative residual.

SELECT
    u.tenant_id,
    u.source_id AS insight_source_id,
    MD5(concat(
        u.tenant_id, '-',
        u.source_id, '-',
        coalesce(u.user_id, ''), '-',
        toString(toDate(parseDateTimeBestEffortOrNull(u.date)))
    )) AS unique_key,
    u.user_id,
    coalesce(u.email_address, '') AS user_name,
    coalesce(u.email_address, '') AS email,
    if(coalesce(u.email_address, '') != '',
       lower(u.email_address),
       lower(u.user_id)) AS person_key,
    toDate(parseDateTimeBestEffortOrNull(u.date)) AS date,
    CAST(NULL AS Nullable(Int64)) AS direct_messages,
    CAST(NULL AS Nullable(Int64)) AS group_chat_messages,
    -- #266: total - channel = DMs + MPIMs (everything posted outside channels).
    toInt64(greatest(
        coalesce(u.messages_posted_count, 0) - coalesce(u.channel_messages_posted_count, 0),
        0
    )) AS direct_and_group_messages,
    coalesce(u.messages_posted_count, 0) AS total_chat_messages,
    u.channel_messages_posted_count AS channel_posts,
    CAST(NULL AS Nullable(Int64)) AS channel_replies,
    CAST(NULL AS Nullable(Int64)) AS urgent_messages,
    CAST(NULL AS Nullable(String)) AS report_period,
    now() AS collected_at,
    'insight_slack' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM {{ source('bronze_slack', 'users_details') }} AS u
WHERE u.user_id IS NOT NULL
  AND u.user_id != ''
  AND parseDateTimeBestEffortOrNull(u.date) IS NOT NULL
{% if is_incremental() %}
  -- Watermark on the source EXTRACT time, not the business date (see zoom model header for
  -- the backfill-strand failure mode this fixes — Slack chat had the same gap on virtuozzo,
  -- bronze from Jan but staging only from late Mar). Re-pulled rows carry a fresh
  -- `_airbyte_extracted_at`, so reprocess every business date touched by a recent extract.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(parseDateTimeBestEffortOrNull(u.date)) IN (
      SELECT DISTINCT toDate(parseDateTimeBestEffortOrNull(date))
      FROM {{ source('bronze_slack', 'users_details') }}
      WHERE _airbyte_extracted_at
            > (SELECT max(_airbyte_extracted_at) FROM {{ source('bronze_slack', 'users_details') }}) - INTERVAL 3 DAY
    )
  )
{% endif %}
