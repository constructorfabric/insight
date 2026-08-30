{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'person_email', 'activity_date'],
    partition_by='toYYYYMM(activity_date)',
    schema=var('gold_database'),
    settings={'allow_nullable_key': 1},
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- Files a person shared, split by who the recipients were: one row per share
-- scope per person, day and tool, so the combined count is a fold over rows and
-- the split is a breakdown of it.
--
-- INVARIANT: a scope the source did not report leaves no row, so an unreported
-- scope stays unknown instead of reading zero.

SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    'internal' AS scope,
    'Internal' AS scope_label,
    files_shared_internal AS files_shared
FROM {{ ref('collab_activity') }}
WHERE files_shared_internal IS NOT NULL

UNION ALL

SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    'external' AS scope,
    'External' AS scope_label,
    files_shared_external AS files_shared
FROM {{ ref('collab_activity') }}
WHERE files_shared_external IS NOT NULL
