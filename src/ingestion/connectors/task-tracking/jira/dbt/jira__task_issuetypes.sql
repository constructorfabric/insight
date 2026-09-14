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
-- `issue_kind` comes from the operator's `task_value_map` binding alone,
-- keyed by the Jira issue-type ID (value_id = toString(id)); an unmapped id
-- is `unknown` and there is no name-list fallback. A type rename would
-- silently re-bucket history under a name key, which is worse than an
-- explicit `unknown`. `value_display` carries the name the decision was made
-- against; `assert_jira_type_map_display_matches_catalog` flags the drift.
--
-- View, not table: bronze `jira_issuetypes` is MergeTree (full_refresh +
-- overwrite), so the current state of bronze is the current state of staging.

WITH authored AS (
    SELECT
        tenant_id,
        insight_source_id,
        value_id,
        canonical_value
    FROM {{ source('config', 'task_value_map') }} FINAL
    WHERE data_source = 'jira'
      AND field_id = 'type'
      AND is_deleted = 0
      AND valid_from <= now64(3)
      -- Only kinds the silver contract accepts.
      AND canonical_value IN ('bug', 'other', 'unknown')
    ORDER BY valid_from DESC, recorded_at DESC
    LIMIT 1 BY tenant_id, insight_source_id, value_id
)

SELECT
    s.unique_key                                                AS unique_key,
    s.source_id                                                 AS insight_source_id,
    CAST('jira' AS String)                                      AS data_source,
    toString(s.id)                                              AS issue_type_id,
    s.name                                                      AS issue_type_name,
    nullIf(toString(s.untranslatedName), '')                    AS untranslated_name,
    -- Under join_use_nulls=0 an unmatched row reads '' from the map, so the
    -- nullIf turns a miss into 'unknown', never ''.
    -- CAST off LowCardinality: `union_by_tag` unions this with the other
    -- sources' branches, and one differing type fails the shared class.
    CAST(COALESCE(
        nullIf(toString(a.canonical_value), ''),
        'unknown'
    ) AS String)                                                AS issue_kind,
    toDateTime64(s._airbyte_extracted_at, 3)                    AS collected_at,
    -- Refreshed per run, not per sync: the kind comes from operator config, so
    -- a classification change must reach silver's incremental filter without
    -- waiting for the connector to re-sync bronze.
    toUnixTimestamp64Milli(now64(3))                            AS _version
FROM {{ source('bronze_jira', 'jira_issuetypes') }} s
-- `jira_issuetypes` bronze = MergeTree (full_refresh + overwrite), FINAL not supported.
LEFT JOIN authored AS a
    ON a.tenant_id = s.tenant_id
    AND a.insight_source_id = s.source_id
    AND a.value_id = toString(s.id)
