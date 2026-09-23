{{ config(
    materialized='table',
    schema='silver',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    tags=['silver']
) }}

-- depends_on: {{ ref('bamboohr__person_absences') }}

SELECT
    insight_tenant_id,
    unique_key,
    account_source_type,
    account_source_id,
    account_id,
    start_date,
    end_date
FROM (
    {{ union_by_tag('silver:class_person_absences', dedup_version_col=none) }}
)
