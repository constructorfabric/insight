{{ config(
    materialized='view',
    schema='insight',
    alias='ai_metric_observations',
    tags=['gold']
) }}

SELECT
    tenant_id,
    source_key,
    entity_type,
    entity_id,
    metric_date,
    observed_at,
    measure_key,
    contribution AS value,
    subject_key,
    dimensions
FROM {{ ref('ai_metric_evidence') }}
