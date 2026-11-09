{{ config(
    materialized='table',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    query_settings={
        'max_bytes_before_external_group_by': 2000000000,
        'max_bytes_before_external_sort': 2000000000,
    },
    tags=['youtrack', 'staging', 'silver:class_task_field_text']
) }}

-- Bodies of long-text fields, addressed by a hash of their content within one
-- tenant's source; `youtrack__task_field_history` carries the hash and a prefix. Every
-- body the history can hold is the display side of a snapshot or activity pair.

WITH observed_bodies AS (
    SELECT s.tenant_id AS tenant_id, s.insight_source_id AS insight_source_id, p.2 AS content
    FROM {{ ref('youtrack__issue_field_snapshot') }} AS s
    INNER JOIN {{ ref('youtrack__long_text_fields') }} AS f USING (tenant_id, insight_source_id, field_id)
    ARRAY JOIN s.pairs AS p
),

changed_bodies AS (
    SELECT a.tenant_id AS tenant_id, a.insight_source_id AS insight_source_id, p.2 AS content
    FROM {{ ref('youtrack__activities') }} AS a
    INNER JOIN {{ ref('youtrack__long_text_fields') }} AS f USING (tenant_id, insight_source_id, field_id)
    ARRAY JOIN arrayConcat(a.before_pairs, a.after_pairs) AS p
),

bodies AS (
    SELECT tenant_id, insight_source_id, content FROM observed_bodies
    UNION ALL
    SELECT tenant_id, insight_source_id, content FROM changed_bodies
)

SELECT
    concat(ifNull(tenant_id, ''), '-', insight_source_id, '-youtrack-', text_id) AS unique_key,
    ifNull(tenant_id, '')                                 AS tenant_id,
    insight_source_id,
    CAST('youtrack' AS String)                            AS data_source,
    text_id,
    CAST('markdown' AS String)                            AS content_form,
    any(content)                                          AS content,
    toUInt64(toUnixTimestamp64Milli(now64(3)))            AS _version
FROM (
    SELECT tenant_id, insight_source_id, {{ task_text_id('content') }} AS text_id, content
    FROM bodies
    WHERE content != ''
)
GROUP BY tenant_id, insight_source_id, text_id
