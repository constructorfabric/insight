{{ config(
    tags=['connector_quality', 'slack'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'Slack bronze rows the pipeline drops on the way to metrics',
        'domain': 'collab',
        'category': 'source_integrity',
        'tier': 'warn',
        'remediation': 'problem=refused_day_row: a row with no user_id or no parseable date. The declarative connector (before the Python CDK rewrite) stored a day Slack refused with {"ok": false} this way; slack__collab_chat_activity drops it. Read the failing sync for the Slack error code, then delete the row from bronze_slack.users_details. problem=missing_counter: Slack sent a member row without messages_posted_count or channel_messages_posted_count (or the destination NULLed it, see _airbyte_meta.changes); staging drops the row rather than read it as 0 messages, so that member-day is missing from metrics. problem=member_without_email: a non-guest member Slack reports with no email_address. It reaches silver keyed by user_id, but collab_metric_evidence keeps only email-keyed rows, so this member has no Slack metrics. Check the member in Slack admin (bot, deactivated or Slack Connect account).'
    }
) }}
SELECT
    'refused_day_row' AS problem,
    unique_key,
    user_id,
    date,
    1 AS days
FROM {{ source('bronze_slack', 'users_details') }} FINAL
WHERE nullIf(user_id, '') IS NULL
   OR parseDateTimeBestEffortOrNull(date) IS NULL

UNION ALL

SELECT
    'missing_counter' AS problem,
    unique_key,
    user_id,
    date,
    1 AS days
FROM {{ source('bronze_slack', 'users_details') }} FINAL
WHERE nullIf(user_id, '') IS NOT NULL
  AND parseDateTimeBestEffortOrNull(date) IS NOT NULL
  AND (messages_posted_count IS NULL OR channel_messages_posted_count IS NULL)

UNION ALL

SELECT
    'member_without_email' AS problem,
    any(unique_key) AS unique_key,
    user_id,
    max(date) AS date,
    count() AS days
FROM {{ source('bronze_slack', 'users_details') }} FINAL
WHERE nullIf(user_id, '') IS NOT NULL
  AND NOT coalesce(is_guest, false)
  AND nullIf(trimBoth(email_address), '') IS NULL
GROUP BY user_id
