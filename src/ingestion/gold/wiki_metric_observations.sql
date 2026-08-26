{{ metric_observations_table() }}

SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    '' AS account_source_type,
    '' AS account_source_id,
    '' AS account_id,
    metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    measure_key,
    toNullable(sum(contribution)) AS value,
    CAST(NULL AS Nullable(String)) AS subject_key,
    dimensions
FROM {{ ref('wiki_metric_evidence') }}
GROUP BY tenant_id, source_key, entity_type, entity_id, metric_date, measure_key, dimensions
