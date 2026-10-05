-- depends_on: {{ ref('github__deployments') }}
{{ config(
    materialized='incremental',
    full_refresh=false,
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['silver']
) }}

-- INVARIANT: like class_git_ci_runs, this accumulates past the source API's
-- retention window — never full-refresh it. Outcome lives in
-- class_git_deployment_events; a deployment without an event is pending.
SELECT candidate.* FROM (
    {{ union_by_tag('silver:class_git_deployments') }}
) AS candidate
{{ silver_incremental_watermark(['tenant_id', 'source_id', 'data_source']) }}
