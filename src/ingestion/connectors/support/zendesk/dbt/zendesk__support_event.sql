-- depends_on: {{ ref('zendesk__bronze_promoted') }}
-- depends_on: {{ ref('zendesk__support_agent') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='silver',
    tags=['zendesk', 'silver']
) }}

-- =====================================================================
-- EVENT-GRAIN support fact — actor-attributed Ticket Audit events.
-- =====================================================================
-- Source: bronze_zendesk.support_ticket_events — ONE row per Zendesk audit,
-- carrying author_id (the ACTOR) and the nested `events` array as JSON. Here we
-- explode that array (arrayJoin) into one Silver row per relevant event, so a
-- single audit legitimately yields both a public_comment and a solved.
--
-- Attribution is strictly on the ACTOR (audit.author_id → agent → person),
-- never the assignee (PRD §2). Customer / end-user authors drop out via the
-- INNER JOIN to internal agents (zendesk__support_agent).
--
-- event_type classification (closes open-Q "updates = all or excl comments"):
--   Comment, public=true   → public_comment
--   Comment, public=false  → private_comment
--   Change, status=solved  → solved        (status→solved is counted ONCE, here)
--   Change, anything else  → update         (other field changes; NOT solved)
-- Non-Comment/Change audit events (Create / Notification / …) are dropped.
SELECT
    e.tenant_id,
    e.source_id                                   AS insight_source_id,
    -- per-event key (audit id + Zendesk event id) so multi-event audits don't collide
    concat('zendesk-', toString(e.audit_id), '-', JSONExtractString(e.ev, 'id')) AS unique_key,
    'zendesk'                                     AS data_source,
    concat('zendesk-', toString(e.ticket_id))     AS ticket_key,
    e.ticket_id                                   AS source_ticket_id,
    ag.person_key                                 AS actor_person_key,     -- KEY OF ATTRIBUTION
    e.author_id                                   AS actor_source_id,
    multiIf(
        JSONExtractString(e.ev, 'type') = 'Comment' AND JSONExtractBool(e.ev, 'public'), 'public_comment',
        JSONExtractString(e.ev, 'type') = 'Comment',                                     'private_comment',
        JSONExtractString(e.ev, 'type') = 'Change'
            AND JSONExtractString(e.ev, 'field_name') = 'status'
            AND JSONExtractString(e.ev, 'value') = 'solved',                             'solved',
        'update'
    )                                             AS event_type,
    -- only meaningful for comment events, else NULL (honest)
    if(JSONExtractString(e.ev, 'type') = 'Comment',
       toUInt8(JSONExtractBool(e.ev, 'public')), CAST(NULL AS Nullable(UInt8))) AS is_public,
    parseDateTimeBestEffortOrNull(e.created_at)   AS occurred_at,
    toDate(parseDateTimeBestEffortOrNull(e.created_at)) AS metric_date,
    now()                                         AS collected_at,
    toUnixTimestamp64Milli(now64())               AS _version
FROM (
    SELECT
        a.tenant_id, a.source_id, a.audit_id, a.ticket_id, a.author_id, a.created_at, ev
    FROM (
        -- Read-time dedup of append-only RMT bronze (ADR-0001): one row per audit.
        SELECT * FROM {{ source('bronze_zendesk', 'support_ticket_events') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY unique_key
    ) a
    -- explode the audit's events[] JSON array into one row per event.
    -- coalesce(): bronze `events` is Nullable(String); ClickHouse refuses to
    -- ARRAY JOIN a Nullable(Array(...)) ("Nested type cannot be inside
    -- Nullable", code 43). Defaulting NULL → '[]' yields an empty array (no
    -- exploded rows) — the honest behaviour for an audit with no events.
    ARRAY JOIN JSONExtractArrayRaw(coalesce(a.events, '[]')) AS ev
    -- keep only the event types that map to a metric
    WHERE JSONExtractString(ev, 'type') IN ('Comment', 'Change')
) e
INNER JOIN {{ ref('zendesk__support_agent') }} ag
        -- tenant+source in the join key: native Zendesk agent ids collide across
        -- instances, so id-only would cross-attribute in a multi-source store.
        ON ag.tenant_id = e.tenant_id
       AND ag.insight_source_id = e.source_id
       AND ag.source_agent_id = e.author_id
WHERE ag.person_key != ''
{% if is_incremental() %}
  AND toDate(parseDateTimeBestEffortOrNull(e.created_at)) > (
      SELECT coalesce(max(metric_date), toDate('1970-01-01')) - INTERVAL 3 DAY FROM {{ this }}
  )
{% endif %}
