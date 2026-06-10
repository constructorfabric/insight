{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['zendesk-audits-pending']
) }}

{#- NO `ref()` to zendesk models on purpose: a ref() edge would make this a
    DOWNSTREAM node of a `zendesk`-tagged model and `tag:zendesk+` would pull
    it into the active run (and fail — support_ticket_events does not exist
    yet). It reads bronze via source() only and is tagged
    `zendesk-audits-pending`, so it is fully excluded until activated. The
    agent map is joined from the raw bronze source here; on activation switch
    it to `ref('zendesk__support_agent')` + add the two `-- depends_on:` lines
    and retag `['zendesk', 'silver']`. -#}

-- =====================================================================
-- EVENT-GRAIN support fact — actor-attributed Ticket Audit events.
-- =====================================================================
-- NOT YET ACTIVE. Tagged `zendesk-audits-pending` (NOT `zendesk`), so it is
-- excluded from the `tag:zendesk+` reconcile run until the connector emits the
-- `support_ticket_events` (Ticket Audits) stream. ACTIVATION CHECKLIST:
--   1. Enable the support_ticket_events stream in connector.yaml
--      (SubstreamPartitionRouter over tickets; one record per audit *change*,
--       NOT per audit — an audit carries comment + status + field changes at
--       once, so the grain must be the change, see point 2).
--   2. Confirm bronze columns and finalise the mapping below
--      (audit_id, change_id, ticket_id, author_id, event_type, public, created_at).
--      unique_key = data_source + audit_id + change_id (per-change uniqueness)
--      so one audit can legitimately emit both `public_comment` and `solved`.
--   3. Retag this model `['zendesk', 'silver']` and point
--      zendesk__support_activity at it (aggregate to person × date).
--
-- One row per audit *change*. Attribution is strictly on the ACTOR
-- (author_id → agent → person), never the assignee (PRD §2). Customer/
-- end-user authors fall out via the INNER JOIN to internal agents.
-- =====================================================================
SELECT
    ev.tenant_id,
    ev.source_id                                  AS insight_source_id,
    'zendesk'                                     AS data_source,
    -- per-change key so multi-change audits don't collide (one audit can carry
    -- comment + status + field changes). ADR-0001: column named unique_key.
    concat('zendesk-', toString(ev.audit_id), '-', toString(ev.change_id)) AS unique_key,
    concat('zendesk-', toString(ev.ticket_id))    AS ticket_key,
    ev.ticket_id                                  AS source_ticket_id,
    a.person_key                                  AS actor_person_key,     -- KEY OF ATTRIBUTION
    ev.author_id                                  AS actor_source_id,
    -- update | public_comment | private_comment | solved | reopened | assigned
    ev.event_type,
    -- only meaningful for comment events, else NULL (honest)
    if(ev.event_type IN ('public_comment', 'private_comment'),
       toUInt8(ev.public), CAST(NULL AS Nullable(UInt8)))                  AS is_public,
    parseDateTimeBestEffortOrNull(ev.created_at)  AS occurred_at,
    toDate(parseDateTimeBestEffortOrNull(ev.created_at)) AS metric_date,
    now()                                         AS collected_at,
    toUnixTimestamp64Milli(now64())               AS _version
FROM (
    -- Read-time dedup of append-only RMT bronze (ADR-0001).
    SELECT * FROM {{ source('bronze_zendesk', 'support_ticket_events') }}
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
) ev
-- Agent map from raw bronze (see header: source(), not ref(), so this stays
-- out of tag:zendesk+). Internal agents only — customer authors drop out.
INNER JOIN (
    SELECT agent_id, lower(email) AS person_key
    FROM {{ source('bronze_zendesk', 'support_agents') }}
    WHERE email IS NOT NULL AND email != ''
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY unique_key
) a ON a.agent_id = ev.author_id
