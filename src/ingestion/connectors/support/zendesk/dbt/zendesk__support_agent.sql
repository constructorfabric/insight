-- depends_on: {{ ref('zendesk__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['zendesk', 'silver:dim_support_agent']
) }}

-- Zendesk slice of the cross-vendor support-agent dimension. Maps a native
-- agent id to a single Insight person via lower(email) — the same join key as
-- every other source (see silver/_shared identity-resolution). Rows whose
-- email is blank are dropped: an agent we cannot resolve to a person must not
-- silently absorb activity (visible gap, not a silent one — see PRD §5).
SELECT
    tenant_id,
    source_id AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-', agent_id)) AS unique_key,
    'zendesk'                       AS data_source,
    agent_id                        AS source_agent_id,
    lower(email)                    AS person_key,        -- FK → insight.people
    email,
    display_name,
    -- Canonical role across vendors. Zendesk: admin | agent | end-user;
    -- light agents come through as 'agent' from the users endpoint filter.
    multiIf(
        role = 'admin',    'admin',
        role = 'agent',    'agent',
        role = 'end-user', 'end_user',
        coalesce(role, '')
    )                               AS role_canonical,
    group_id                        AS group_source_id,
    group_name,
    is_active,
    now()                           AS collected_at,
    toUnixTimestamp64Milli(now64()) AS _version
FROM (
    -- Read-time dedup of append-only RMT bronze (ADR-0001): keep the latest
    -- extract per unique_key so re-delivered rows never duplicate downstream.
    SELECT * FROM {{ source('bronze_zendesk', 'support_agents') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
)
WHERE email IS NOT NULL AND email != ''
