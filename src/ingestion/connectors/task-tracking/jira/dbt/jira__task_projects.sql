-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='jira__task_projects',
    schema='staging',
    tags=['jira', 'silver:class_task_projects']
) }}

-- View, not table: bronze `jira_projects` is MergeTree (full_refresh + overwrite),
-- so the current state of bronze is the current state of staging — no incremental
-- accumulation needed. Silver `class_task_projects` is RMT(_version), reads via FINAL.

SELECT
    p.unique_key                                AS unique_key,
    p.source_id                                 AS insight_source_id,
    CAST('jira' AS String)                      AS data_source,
    toString(p.project_id)                      AS project_id,
    p.project_key                               AS project_key,
    p.name                                      AS name,
    -- `lead_account_id` column absent in bronze (Airbyte auto-detect skipped the AddFields
    -- column because values were all null in sampled data). Stub until schema is fixed.
    CAST(NULL AS Nullable(String))              AS lead_id,
    p.project_type                              AS project_type,
    p.style                                     AS project_style,
    -- `p.archived` is `Nullable(Bool)` in bronze. `toString(true) = 'true'`, which
    -- `toUInt8OrNull` cannot parse — the old expression silently produced 100% NULL.
    CAST(p.archived AS Nullable(UInt8))         AS archived,
    now64(3)                                    AS collected_at,
    toUnixTimestamp64Milli(now64(3))            AS _version
FROM {{ source('bronze_jira', 'jira_projects') }} p
-- `jira_projects` bronze = MergeTree (full_refresh + overwrite), FINAL not supported.
