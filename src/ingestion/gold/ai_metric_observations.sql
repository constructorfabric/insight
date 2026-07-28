{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['source_key', 'measure_key', 'entity_id', 'metric_date'],
    schema='insight',
    alias='ai_metric_observations',
    tags=['gold'],
    query_settings={
        'max_memory_usage': 1610612736,
        'max_threads': 4,
        'max_bytes_before_external_group_by': 805306368,
        'max_bytes_before_external_sort': 805306368
    }
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
