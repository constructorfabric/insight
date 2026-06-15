-- depends_on: {{ ref('zulip_proxy__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    on_schema_change='append_new_columns',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['zulip_proxy', 'silver:class_collab_chat_activity']
) }}

-- Zulip-Proxy chat activity rolled up per (tenant, source, sender email, date).
--
-- Source granularity: `bronze_zulip_proxy.messages` already arrives aggregated
-- by the proxy — one row per `uniq` carries a `count` of messages for a single
-- `sender_id` within one aggregation bucket anchored at `created_at`. The
-- bucket period is opaque to this model; we treat `created_at` as a timestamp
-- and bucket by date for the Silver grain.
--
-- Identity model: emails come from `bronze_zulip_proxy.users` joined by
-- `sender_id = id`. We use FINAL to collapse the RMT bronze duplicates of the
-- user directory (the directory is re-pulled full-refresh on every sync), and
-- we filter out messages where the sender email is missing — anonymous /
-- system-bot senders cannot be joined to identity at Silver and add noise.
--
-- We do NOT have direct messages / channel posts / channel replies splits —
-- the proxy only exposes total counts per sender per bucket. Sibling
-- collaboration sources (M365, Slack) split chat activity into DM vs channel;
-- for Zulip we honestly emit NULL on those columns and put the total into
-- `total_chat_messages`.

SELECT
    m.tenant_id,
    m.source_id AS insight_source_id,
    MD5(concat(
        m.tenant_id, '-',
        m.source_id, '-',
        lower(u.email), '-',
        toString(toDate(parseDateTimeBestEffortOrNull(m.created_at)))
    )) AS unique_key,
    lower(u.email) AS user_id,
    coalesce(any(u.full_name), '') AS user_name,
    lower(u.email) AS email,
    lower(u.email) AS person_key,
    toDate(parseDateTimeBestEffortOrNull(m.created_at)) AS date,
    CAST(NULL AS Nullable(Int64)) AS direct_messages,
    CAST(NULL AS Nullable(Int64)) AS group_chat_messages,
    CAST(NULL AS Nullable(Int64)) AS direct_and_group_messages,
    -- Proxy reports a single aggregated count per (sender, bucket).
    toInt64(sum(coalesce(m.count, 0))) AS total_chat_messages,
    CAST(NULL AS Nullable(Int64)) AS channel_posts,
    CAST(NULL AS Nullable(Int64)) AS channel_replies,
    CAST(NULL AS Nullable(Int64)) AS urgent_messages,
    CAST(NULL AS Nullable(String)) AS report_period,
    now() AS collected_at,
    'insight_zulip_proxy' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version
FROM (
    -- Drop bronze re-emit duplicates of the same aggregate bucket. RMT
    -- normally collapses these on FINAL, but the incremental SUM(count)
    -- below must not double-count if the table has not been OPTIMIZE'd
    -- yet — pre-dedup by `uniq` (the proxy's primary key).
    SELECT *
    FROM {{ source('bronze_zulip_proxy', 'messages') }}
    WHERE parseDateTimeBestEffortOrNull(created_at) IS NOT NULL
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY tenant_id, source_id, uniq
) AS m
LEFT JOIN {{ source('bronze_zulip_proxy', 'users') }} AS u FINAL
    ON u.tenant_id = m.tenant_id
    AND u.source_id = m.source_id
    AND u.id = m.sender_id
WHERE u.email IS NOT NULL AND u.email != ''
{% if is_incremental() %}
  -- Watermark on the source EXTRACT time, not the business date (see zoom model header
  -- for the backfill-strand failure mode this fixes). Re-pulled rows carry a fresh
  -- `_airbyte_extracted_at`, so reprocess every business date touched by a recent extract.
  AND (
    (SELECT count() FROM {{ this }}) = 0
    OR toDate(parseDateTimeBestEffortOrNull(m.created_at)) IN (
      SELECT DISTINCT toDate(parseDateTimeBestEffortOrNull(created_at))
      FROM {{ source('bronze_zulip_proxy', 'messages') }}
      WHERE _airbyte_extracted_at
            > (SELECT max(_airbyte_extracted_at) FROM {{ source('bronze_zulip_proxy', 'messages') }}) - INTERVAL 3 DAY
    )
  )
{% endif %}
GROUP BY
    m.tenant_id,
    m.source_id,
    lower(u.email),
    toDate(parseDateTimeBestEffortOrNull(m.created_at))
