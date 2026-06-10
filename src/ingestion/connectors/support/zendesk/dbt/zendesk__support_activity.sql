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
    tags=['zendesk', 'silver:class_support_activity']
) }}

-- =====================================================================
-- Zendesk → person × date support-activity (the "activity signal").
-- =====================================================================
-- This is the rollup that sits NEXT TO Collaboration metrics
-- (silver/collaboration/class_collab_*_activity) — same grain
-- (person_key × date), same union mechanism. Gold reads
-- class_support_activity, NOT the event grain.
--
-- COLUMN STATUS
--   csat_good / csat_total  — LIVE now. Sourced from
--       zendesk_satisfaction_ratings, attributed to the ticket's *assignee*
--       (the agreed CSAT exception to actor-attribution — the rating is set
--       by the customer and bound to assignee by Zendesk; see PRD §4).
--   updates / public_comments / private_comments / solved — honest NULL until
--       the Ticket Audits stream (support_ticket_events) is enabled. These are
--       actor-attributed and CANNOT be derived from the ticket snapshot
--       without double-/mis-counting (PRD §2). When the stream lands, build
--       zendesk__support_event (event grain) and FULL-merge its per-actor
--       per-day counts into this model by (person_key, date).
--   kb_articles_created — honest NULL until the Guide/Help-Center stream
--       (articles) is enabled.
-- NULL (not 0) is deliberate — "not measured yet" ≠ "measured zero"
-- (platform honest-nulls convention, see bullet-views-honest-nulls).
-- =====================================================================

WITH csat AS (
    SELECT
        r.tenant_id,
        r.source_id,
        a.person_key,
        a.email,
        toDate(parseDateTimeBestEffortOrNull(r.created_at))                       AS date,
        countIf(startsWith(lower(coalesce(r.score, '')), 'good'))                 AS csat_good,
        countIf(startsWith(lower(coalesce(r.score, '')), 'good')
                OR startsWith(lower(coalesce(r.score, '')), 'bad'))               AS csat_total
    FROM (
        -- Read-time dedup BEFORE the countIf (ADR-0001). Critical here: this
        -- model AGGREGATES, so a re-delivered rating (incremental 3-day
        -- overlap) would inflate csat_good/csat_total and the bad value would
        -- be baked into the row — RMT(_version) on the output cannot undo it.
        -- Keep one row per rating unique_key (latest extract).
        SELECT * FROM {{ source('bronze_zendesk', 'zendesk_satisfaction_ratings') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY unique_key
    ) r
    INNER JOIN {{ ref('zendesk__support_agent') }} a
            ON a.source_agent_id = r.assignee_id
    WHERE a.person_key != ''
    GROUP BY r.tenant_id, r.source_id, a.person_key, a.email, date
)
SELECT
    tenant_id,
    source_id                       AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-zendesk-', person_key, '-', toString(date))) AS unique_key,
    'zendesk'                       AS data_source,
    person_key,                                            -- FK → insight.people
    email,
    date,
    -- actor-attributed activity (pending support_ticket_events stream)
    CAST(NULL AS Nullable(UInt32)) AS updates,
    CAST(NULL AS Nullable(UInt32)) AS public_comments,
    CAST(NULL AS Nullable(UInt32)) AS private_comments,
    CAST(NULL AS Nullable(UInt32)) AS solved,
    -- KB authoring (pending Guide/Help-Center stream)
    CAST(NULL AS Nullable(UInt32)) AS kb_articles_created,
    -- CSAT (live) — assignee-attributed
    csat_good,
    csat_total,
    now()                           AS collected_at,
    toUnixTimestamp64Milli(now64()) AS _version
FROM csat
{% if is_incremental() %}
WHERE (
    (SELECT max(date) FROM {{ this }}) IS NULL
    OR date > (SELECT max(date) - INTERVAL 3 DAY FROM {{ this }})
)
{% endif %}
