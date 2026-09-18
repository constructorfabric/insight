{{ config(
    materialized='view',
    alias='jira__task_issuetypes',
    schema='staging',
    tags=['jira', 'staging', 'silver:class_task_issuetypes']
) }}

-- Per-source issue-type dimension; unioned into `silver.class_task_issuetypes`
-- via `union_by_tag`. Raw vendor catalogue only: `untranslatedName` is the
-- type's language-independent name, `name` the display label. Classification
-- (the issue kind) is not decided here — gold resolves it from
-- `config.field_value_map` at its own build, so a mapping change never
-- requires a silver rebuild.
--
-- View, not table: the current state of bronze is the current state of
-- staging. Bronze is ReplacingMergeTree, so the read carries FINAL.

SELECT
    s.unique_key                                                AS unique_key,
    s.source_id                                                 AS insight_source_id,
    CAST('jira' AS String)                                      AS data_source,
    toString(s.id)                                              AS issue_type_id,
    s.name                                                      AS issue_type_name,
    nullIf(toString(s.untranslatedName), '')                    AS untranslated_name,
    toDateTime64(s._airbyte_extracted_at, 3)                    AS collected_at,
    -- Refreshed per run, not per sync, so a contract change (a healed column,
    -- a rename already synced into bronze) reaches silver's incremental filter
    -- without waiting for the connector to re-sync bronze.
    toUnixTimestamp64Milli(now64(3))                            AS _version
FROM {{ source('bronze_jira', 'jira_issuetypes') }} s FINAL
