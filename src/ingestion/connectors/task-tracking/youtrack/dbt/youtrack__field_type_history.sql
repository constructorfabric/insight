{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

SELECT insight_source_id, field_id, field_type, observed_at,
    multiIf(field_type = '', 'unknown', endsWith(field_type, '[*]'), 'multi', 'single') AS observed_cardinality,
    lagInFrame(toNullable(observed_at)) OVER w AS previously_observed_at,
    lagInFrame(toNullable(field_type)) OVER w AS previously_observed_type
FROM {{ ref('youtrack__field_observations') }} FINAL
WINDOW w AS (PARTITION BY insight_source_id, field_id ORDER BY observed_at
             ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW)
