-- depends_on: {{ ref('zendesk__bronze_promoted') }}
-- depends_on: {{ ref('zendesk__support_agent') }}
-- depends_on: {{ ref('zendesk__support_event') }}
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
-- The rollup that sits NEXT TO Collaboration metrics
-- (silver/collaboration/class_collab_*_activity) — same grain (person_key ×
-- date), same union mechanism. Gold reads class_support_activity, not events.
--
-- COLUMN STATUS
--   updates / public_comments / private_comments / solved — LIVE: counted from
--       zendesk__support_event (Ticket Audits), attributed to the ACTOR. 0 is a
--       genuine measured zero now (the audit stream is ingested), not a stub.
--   csat_good / csat_total — LIVE from zendesk_satisfaction_ratings, attributed
--       to the ticket *assignee* (agreed CSAT exception to actor-attribution,
--       PRD §4 — the rating is set by the customer and bound to assignee).
--   kb_articles_created — honest NULL until the Guide/Help-Center stream lands
--       ("not measured yet" ≠ "measured zero", honest-nulls convention).
--
-- Activity (actor-attributed) and CSAT (assignee-attributed) are different
-- attributions but the SAME grain (person × date), so we UNION the two
-- contributions and sum per (person, date).
-- =====================================================================

WITH events AS (
    SELECT
        tenant_id,
        insight_source_id                         AS source_id,
        actor_person_key                          AS person_key,
        metric_date                               AS date,
        countIf(event_type = 'update')            AS updates,
        countIf(event_type = 'public_comment')    AS public_comments,
        countIf(event_type = 'private_comment')   AS private_comments,
        countIf(event_type = 'solved')            AS solved
    FROM {{ ref('zendesk__support_event') }}
    WHERE actor_person_key != ''
    GROUP BY tenant_id, source_id, person_key, date
),
csat AS (
    SELECT
        r.tenant_id                                                           AS tenant_id,
        r.source_id                                                           AS source_id,
        a.person_key                                                          AS person_key,
        toDate(parseDateTimeBestEffortOrNull(r.created_at))                   AS date,
        countIf(startsWith(lower(coalesce(r.score, '')), 'good'))             AS csat_good,
        countIf(startsWith(lower(coalesce(r.score, '')), 'good')
                OR startsWith(lower(coalesce(r.score, '')), 'bad'))           AS csat_total
    FROM (
        -- Read-time dedup BEFORE the countIf (ADR-0001): a re-delivered rating
        -- would otherwise inflate the CSAT counts, baked into the row.
        SELECT * FROM {{ source('bronze_zendesk', 'zendesk_satisfaction_ratings') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY unique_key
    ) r
    INNER JOIN {{ ref('zendesk__support_agent') }} a
            ON a.source_agent_id = r.assignee_id
    WHERE a.person_key != ''
    GROUP BY r.tenant_id, r.source_id, a.person_key, date
),
merged AS (
    SELECT tenant_id, source_id, person_key, date,
           updates, public_comments, private_comments, solved,
           toUInt32(0) AS csat_good, toUInt32(0) AS csat_total
    FROM events
    UNION ALL
    SELECT tenant_id, source_id, person_key, date,
           toUInt32(0) AS updates, toUInt32(0) AS public_comments,
           toUInt32(0) AS private_comments, toUInt32(0) AS solved,
           csat_good, csat_total
    FROM csat
)
SELECT
    tenant_id,
    source_id                       AS insight_source_id,
    MD5(concat(tenant_id, '-', source_id, '-zendesk-', person_key, '-', toString(date))) AS unique_key,
    'zendesk'                       AS data_source,
    person_key,                                            -- FK → insight.people (= lower(email))
    person_key                      AS email,
    date,
    sum(updates)                    AS updates,
    sum(public_comments)            AS public_comments,
    sum(private_comments)           AS private_comments,
    sum(solved)                     AS solved,
    -- KB authoring (pending Guide/Help-Center stream)
    CAST(NULL AS Nullable(UInt32))  AS kb_articles_created,
    sum(csat_good)                  AS csat_good,
    sum(csat_total)                 AS csat_total,
    now()                           AS collected_at,
    toUnixTimestamp64Milli(now64()) AS _version
FROM merged
{% if is_incremental() %}
WHERE (
    (SELECT max(date) FROM {{ this }}) IS NULL
    OR date > (SELECT max(date) - INTERVAL 3 DAY FROM {{ this }})
)
{% endif %}
GROUP BY tenant_id, source_id, person_key, date
