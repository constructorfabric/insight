-- depends_on: {{ ref('zendesk__support_agent') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    incremental_strategy='delete+insert',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['zendesk']
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
    -- tenant+source in the key, not just the Zendesk ids: audit and event ids
    -- are sequential PER INSTANCE, so an id-only key collides across tenants
    -- and across two Zendesk instances in one install — and since this model
    -- is delete+insert on unique_key, the collision DELETES the other row.
    MD5(concat(e.tenant_id, '-', e.source_id, '-', toString(e.audit_id),
               '-', JSONExtractString(e.ev, 'id'))) AS unique_key,
    'zendesk'                                     AS data_source,
    -- same formula as zendesk__support_ticket.unique_key, so the two join
    MD5(concat(e.tenant_id, '-', e.source_id, '-', toString(e.ticket_id))) AS ticket_key,
    e.ticket_id                                   AS source_ticket_id,
    e.audit_id                                    AS source_audit_id,
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
        a.tenant_id, a.source_id, a.audit_id, a.ticket_id, a.author_id, a.created_at,
        a._airbyte_extracted_at, ev
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
  AND parseDateTimeBestEffortOrNull(e.created_at) IS NOT NULL
{% if is_incremental() %}
  -- Watermark on the source EXTRACT time, not the business date: audits are
  -- fanned out per ticket off the parent's updated_at cursor, so a ticket
  -- created months ago and touched today yields audits with an OLD created_at,
  -- and a business-date boundary that only rises would strand them. Anchored
  -- on this model's last BUILD (collected_at = now() at build), not on
  -- bronze's newest extract: a build at time T consumed every row extracted
  -- before T, so the boundary holds whatever the sync/dbt cadence, where a
  -- bronze anchor strands rows extracted more than 3 days before the newest
  -- extract whenever dbt skipped a few nightly runs.
  AND e._airbyte_extracted_at > (SELECT max(collected_at) FROM {{ this }}) - INTERVAL 3 DAY
{% endif %}
