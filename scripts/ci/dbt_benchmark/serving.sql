SELECT
    'synthetic' AS tenant_id,
    'synthetic_source' AS source_key,
    'person' AS entity_type,
    toString(id % 10000) AS entity_id,
    concat('measure-', toString(id % 4)) AS measure_key,
    toDate('2023-01-01') + toIntervalDay(id % 1095) AS metric_date,
    toString(id) AS record_id,
    id % 17 AS contribution,
    payload
FROM benchmark.input
