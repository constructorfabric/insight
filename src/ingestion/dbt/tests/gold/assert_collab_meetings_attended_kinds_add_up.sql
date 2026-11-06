-- Build-integrity check (untagged → error severity under `dbt build`).
-- The kind split of meetings_attended is a partition of the source count: on
-- every Teams report day the kinds sum to meetings attended plus calls, so the
-- metric's total is the same number it was before the split existed. A
-- violation means a kind arm double-counts, drops a remainder, or a split
-- column outgrew the total it belongs to.
WITH kinds AS (
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        sum(value) AS kinds_total
    FROM {{ ref('collab_metric_observations') }}
    WHERE measure_key = 'meetings_attended'
      AND has(dimensions.2, 'm365')
      AND subject_key = entity_id
    GROUP BY tenant_id, entity_id, metric_date
),
reported AS (
    SELECT
        tenant_id,
        {{ normalized_email('person_key') }} AS entity_id,
        date AS metric_date,
        sum(meetings_attended + ifNull(calls_count, 0)) AS reported_total
    FROM {{ ref('class_collab_meeting_activity') }} FINAL
    WHERE data_source = 'insight_m365'
      AND person_key LIKE '%@%'
      AND date IS NOT NULL
    GROUP BY tenant_id, entity_id, metric_date
)
SELECT
    reported.tenant_id,
    reported.entity_id,
    reported.metric_date,
    reported.reported_total,
    kinds.kinds_total
FROM reported
LEFT JOIN kinds
    ON kinds.tenant_id = reported.tenant_id
   AND kinds.entity_id = reported.entity_id
   AND kinds.metric_date = reported.metric_date
WHERE ifNull(kinds.kinds_total, -1) != reported.reported_total
