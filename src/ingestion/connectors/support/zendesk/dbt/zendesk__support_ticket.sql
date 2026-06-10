-- depends_on: {{ ref('zendesk__bronze_promoted') }}
-- depends_on: {{ ref('zendesk__support_agent') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['zendesk', 'silver:dim_support_ticket']
) }}

-- Zendesk slice of the cross-vendor ticket dimension. CONTEXT ONLY — the
-- `assignee_person_key` here is the CURRENT-snapshot assignee and MUST NOT be
-- used to attribute activity (assignee changes; the updater is often not the
-- assignee — see PRD §2). Activity is attributed to the actor in
-- class_support_activity, not from this table.
SELECT
    t.tenant_id,
    t.source_id                                  AS insight_source_id,
    MD5(concat(t.tenant_id, '-', t.source_id, '-', t.ticket_id)) AS unique_key,
    'zendesk'                                    AS data_source,
    t.ticket_id                                  AS source_ticket_id,
    t.subject,
    -- Zendesk statuses: new | open | pending | hold | solved | closed.
    multiIf(lower(t.status) = 'hold', 'on_hold', lower(t.status)) AS status_canonical,
    lower(coalesce(t.priority, ''))              AS priority_canonical,   -- low | normal | high | urgent
    lower(coalesce(t.ticket_type, ''))           AS type_canonical,       -- question | incident | problem | task
    t.assignee_id                                AS assignee_source_id,
    a.person_key                                 AS assignee_person_key,  -- snapshot only — NOT for attribution
    t.group_id                                   AS group_source_id,
    t.requester_id                               AS requester_source_id,  -- external customer; NOT mapped to insight.people
    t.organization_id                            AS org_source_id,
    parseDateTimeBestEffortOrNull(t.created_at)  AS created_at,
    parseDateTimeBestEffortOrNull(t.updated_at)  AS updated_at,
    parseDateTimeBestEffortOrNull(t.solved_at)   AS solved_at,
    now()                                        AS collected_at,
    toUnixTimestamp64Milli(now64())              AS _version
FROM (
    -- Read-time dedup of append-only RMT bronze (ADR-0001).
    SELECT * FROM {{ source('bronze_zendesk', 'support_tickets') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
) t
LEFT JOIN {{ ref('zendesk__support_agent') }} a
       ON a.source_agent_id = t.assignee_id
