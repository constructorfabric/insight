-- depends_on: {{ ref('zendesk__support_ticket') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

-- Cross-vendor ticket dimension — CONTEXT ONLY (subject, status, priority,
-- type, group). Union of per-source staging tagged `silver:dim_support_ticket`.
-- `assignee_person_key` is a current snapshot and MUST NOT be used to
-- attribute activity (activity lives in class_support_activity, by actor).
SELECT candidate.* FROM (
    {{ union_by_tag('silver:dim_support_ticket') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'insight_source_id', 'data_source']) }}
