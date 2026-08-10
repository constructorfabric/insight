-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='jira__task_issuetypes',
    schema='staging',
    tags=['jira', 'staging', 'silver:class_task_issuetypes']
) }}

-- Per-source issue-type dimension; unioned into `silver.class_task_issuetypes`
-- via `union_by_tag`. Classification reads `untranslatedName`, the type's
-- language-independent name; `name` is the display label.
--
-- View, not table: bronze `jira_issuetypes` is MergeTree (full_refresh +
-- overwrite), so the current state of bronze is the current state of staging.


SELECT
    s.unique_key                                                AS unique_key,
    s.source_id                                                 AS insight_source_id,
    CAST('jira' AS String)                                      AS data_source,
    toString(s.id)                                              AS issue_type_id,
    s.name                                                      AS issue_type_name,
    nullIf(toString(s.untranslatedName), '')                    AS untranslated_name,
    {{ task_issue_kind("coalesce(nullIf(toString(s.untranslatedName), ''), toString(s.name))") }}
                                                                AS issue_kind,
    toDateTime64(s._airbyte_extracted_at, 3)                    AS collected_at,
    -- Refreshed per run, not per sync: the kind comes from configured name
    -- lists, so a classification change must reach silver's incremental filter
    -- without waiting for the connector to re-sync bronze.
    toUnixTimestamp64Milli(now64(3))                            AS _version
FROM {{ source('bronze_jira', 'jira_issuetypes') }} s
-- `jira_issuetypes` bronze = MergeTree (full_refresh + overwrite), FINAL not supported.
