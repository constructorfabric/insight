-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='jira__task_users',
    schema='staging',
    tags=['jira', 'silver:class_task_users']
) }}

-- Per-source staging model; unioned into `silver.class_task_users` via `union_by_tag`.
-- Anchor for identity resolution.
--
-- View, not table: bronze `jira_user` is MergeTree (full_refresh + overwrite),
-- so the current state of bronze is the current state of staging.
-- `_version` is `_airbyte_extracted_at` — deterministic and monotonic.

SELECT
    u.unique_key                                AS unique_key,
    u.source_id                                 AS insight_source_id,
    CAST('jira' AS String)                      AS data_source,
    u.account_id                                AS user_id,
    u.email                                     AS email,
    u.display_name                              AS display_name,
    CAST(NULL AS Nullable(String))              AS username,
    u.account_type                              AS account_type,
    -- Same reason as `archived` in jira__task_projects: `u.active` is `Nullable(Bool)`;
    -- `toUInt8OrNull(toString(...))` was silently producing 100% NULL.
    CAST(u.active AS Nullable(UInt8))           AS is_active,
    toDateTime64(u._airbyte_extracted_at, 3)    AS collected_at,
    toUnixTimestamp64Milli(u._airbyte_extracted_at) AS _version
FROM {{ source('bronze_jira', 'jira_user') }} u
-- `jira_user` bronze = MergeTree (full_refresh + overwrite), FINAL not supported and not needed.
