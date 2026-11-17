{{ config(
    materialized='incremental',
    unique_key='unique_key',
    on_schema_change='append_new_columns',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['m365', 'silver:class_collab_chat_activity']
) }}

-- INVARIANT: privateChatMessageCount counts private-chat messages without splitting
-- one-to-one from group chats; teamChatMessageCount is postMessages + replyMessages.

WITH newest_extract AS (

    SELECT
        tenant_id,
        source_id AS insight_source_id,
        MD5(concat(tenant_id, '-', source_id, '-', lower(coalesce(userPrincipalName, '')), '-', toString(reportRefreshDate))) AS unique_key,
        lower(userPrincipalName) AS user_id,
        lower(userPrincipalName) AS user_name,
        lower(userPrincipalName) AS email,
        if(userPrincipalName IS NOT NULL AND userPrincipalName != '',
           lower(userPrincipalName),
           '') AS person_key,
        toDate(reportRefreshDate) AS date,
        toInt64(privateChatMessageCount) AS direct_messages,
        CAST(NULL AS Nullable(Int64)) AS group_chat_messages,
        toInt64(privateChatMessageCount) AS direct_and_group_messages,
        toInt64(COALESCE(privateChatMessageCount, 0) + COALESCE(teamChatMessageCount, 0)) AS total_chat_messages,
        toInt64(postMessages) AS channel_posts,
        toInt64(replyMessages) AS channel_replies,
        toInt64(urgentMessages) AS urgent_messages,
        reportPeriod AS report_period,
        now() AS collected_at,
        'insight_m365' AS data_source,
        toUnixTimestamp64Milli(now64()) AS _version,
        coalesce(isLicensed, false) = true AS licensed
    FROM {{ source('bronze_m365', 'teams_activity') }} FINAL
    WHERE userPrincipalName IS NOT NULL
      AND userPrincipalName != ''
      AND {{ m365_day_was_reported('teams_activity') }}
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
    -- INVARIANT: Microsoft may return one account's UPN in a different letter case
    -- on another extract; the key is case-insensitive and the newest extract wins.
    -- The license filter runs after this pick, so an older licensed variant never
    -- stands in for a newer unlicensed one.
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key

)

SELECT * EXCEPT (licensed)
FROM newest_extract
-- Drop unlicensed users (guests, ex-employees, service accounts). Their Teams
-- activity inflates team-level collab counters and produces gold rows that
-- match no person. `isLicensed` = "Selected if the user
-- is licensed to use Teams" (MS Graph getTeamsUserActivityUserDetail). Only the
-- teams_activity report exposes this flag; the email/onedrive/sharepoint feeders
-- fall back to `assignedProducts`. A NULL flag is treated as unlicensed and
-- dropped too, per the #736 proposal. See #736.
WHERE licensed
