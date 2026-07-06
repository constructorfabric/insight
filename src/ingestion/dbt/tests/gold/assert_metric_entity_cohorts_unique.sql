-- Build-integrity check (untagged → error severity under `dbt build`).
-- The analytics service peer view joins insight.metric_entity_cohorts_current on
-- (tenant_id, entity_type, entity_id, cohort_key) and assumes exactly one row
-- per key — duplicate rows fan out the join and corrupt peer percentiles.
-- Any returned row is a violation of that contract.
SELECT
    tenant_id,
    entity_type,
    entity_id,
    cohort_key,
    count() AS row_count
FROM {{ ref('metric_entity_cohorts_current') }}
GROUP BY tenant_id, entity_type, entity_id, cohort_key
HAVING count() > 1
