{{ config(
    materialized='table',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    tags=['youtrack', 'staging', 'silver:class_task_field_current_text']
) }}

-- The current body of each long-text field of an issue, from its newest
-- snapshot: a reference into `youtrack__task_field_text`. An empty field has no row.

WITH latest AS (
    SELECT s.tenant_id AS tenant_id, s.insight_source_id AS insight_source_id, s.issue_id AS issue_id, s.field_id AS field_id,
        argMax(s.pairs, s.observed_at) AS pairs
    FROM {{ ref('youtrack__issue_field_snapshot') }} AS s
    INNER JOIN {{ ref('youtrack__long_text_fields') }} AS f USING (tenant_id, insight_source_id, field_id)
    GROUP BY s.tenant_id, s.insight_source_id, s.issue_id, s.field_id
)

SELECT
    concat(ifNull(tenant_id, ''), '-', insight_source_id, '-youtrack-', issue_id, '-', field_id) AS unique_key,
    ifNull(tenant_id, '')                                 AS tenant_id,
    insight_source_id,
    CAST('youtrack' AS String)                            AS data_source,
    issue_id,
    field_id,
    {{ task_text_id('pairs[1].2') }}                      AS text_id,
    toUInt64(toUnixTimestamp64Milli(now64(3)))            AS _version
FROM latest
WHERE length(pairs) > 0 AND pairs[1].2 != ''
