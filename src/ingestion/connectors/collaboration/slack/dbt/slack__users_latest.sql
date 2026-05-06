-- depends_on: {{ ref('slack__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['slack']
) }}

-- Dedup bronze_slack.users_details to one row per Slack user.
-- Identity is (tenant_id, source_id, user_id) so email changes register as
-- attribute updates in the downstream SCD2 snapshot, not as new entities.
-- Bronze has one row per user per day; we keep the latest `date` seen and
-- drop rows with null/empty email.

WITH ranked AS (
    SELECT
        tenant_id,
        source_id,
        user_id,
        email_address AS email,
        is_guest,
        is_billable_seat,
        date,
        row_number() OVER (
            PARTITION BY tenant_id, source_id, user_id
            ORDER BY date DESC
        ) AS rn
    FROM {{ source('bronze_slack', 'users_details') }}
    WHERE email_address IS NOT NULL
      AND email_address != ''
)

SELECT
    tenant_id,
    source_id,
    user_id,
    email,
    is_guest,
    is_billable_seat,
    concat(tenant_id, '-', source_id, '-', user_id) AS unique_key
FROM ranked
WHERE rn = 1
