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

-- The collaboration modalities a person was deliberately active in: one row per
-- modality per person, day and tool, so counting distinct modalities is a
-- distinct count over rows rather than a fold over four flag columns.

SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    'chat' AS modality
FROM {{ ref('collab_activity') }}
WHERE is_chat_active = 1

UNION ALL

SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    'email' AS modality
FROM {{ ref('collab_activity') }}
WHERE is_email_active = 1

UNION ALL

SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    'documents' AS modality
FROM {{ ref('collab_activity') }}
WHERE is_documents_active = 1

UNION ALL

SELECT
    tenant_id,
    person_email,
    activity_date,
    tool,
    'meetings' AS modality
FROM {{ ref('collab_activity') }}
WHERE is_meetings_active = 1
