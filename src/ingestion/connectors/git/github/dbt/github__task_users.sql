{{ config(
    materialized='view',
    alias='github__task_users',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_users']
) }}

-- Per-source user dimension; unioned into `silver.class_task_users` via
-- `union_by_tag`. Gold reads an assignee's address from a row here, and a row
-- exists only for an account with a known address; an account with none keeps
-- its issues under the account id, which attributes only once identity binds
-- it — so the coverage of `github__account_emails` still bounds how much of the
-- tracker reaches a dashboard by e-mail alone.
--
-- `user_id` is the numeric account id as text, not the login: a login is
-- renameable and the identity bridge already keys on the id.
--
-- Several e-mails per account is normal; the earliest observation wins so the
-- choice is stable across runs rather than varying with merge order.

WITH ranked AS (
    SELECT
        tenant_id,
        source_id,
        account_id,
        email,
        observed_at
    FROM {{ ref('github__account_emails') }}
    ORDER BY observed_at ASC, email ASC
    LIMIT 1 BY tenant_id, source_id, account_id
)

SELECT
    CAST(concat(ifNull(tenant_id, ''), ':', source_id, ':github:user:', account_id) AS Nullable(String)) AS unique_key,
    CAST(tenant_id AS Nullable(String))                     AS tenant_id,
    CAST(source_id AS Nullable(String))                     AS insight_source_id,
    CAST('github' AS String)                                AS data_source,
    CAST(account_id AS Nullable(String))                    AS user_id,
    CAST(lower(email) AS Nullable(String))                  AS email,
    CAST(NULL AS Nullable(String))                          AS display_name,
    CAST(NULL AS Nullable(String))                          AS username,
    CAST('user' AS Nullable(String))                        AS account_type,
    CAST(NULL AS Nullable(UInt8))                           AS is_active,
    toDateTime64(observed_at, 3)                            AS collected_at,
    toUnixTimestamp64Milli(toDateTime64(observed_at, 3))    AS _version
FROM ranked
