-- depends_on: {{ ref('chatgpt_team__bronze_promoted') }}
-- Diagnostic: where the two Codex readings of a person-day disagree.
--
-- The vendor publishes Codex usage twice — the usage leaderboard and the
-- session/message aggregate — and neither is a superset of the other. A model
-- that joins one onto the other can only ever show the joining side's
-- key space, so a person-day the other endpoint alone returned would be
-- invisible rather than absent. This view carries both key spaces so the
-- difference is queryable.
--
-- INVARIANT: diagnostic only. This is not a class relation, it feeds no metric,
-- it defines no contract and it must never become a source of active_day.
-- chatgpt_team__ai_dev_usage keeps deciding admission on its own activity
-- counters; nothing here changes what that model emits.
--
-- A view, not a table: it answers a question asked while investigating, it has
-- no consumer that needs it materialised, and a divergence worth alerting on
-- would be a test over this view rather than stored rows.
{{ config(
    materialized='view',
    schema='staging',
    tags=['chatgpt-team']
) }}

WITH
-- INVARIANT: deduped to the JOIN grain, which is narrower than the Bronze key.
-- The leaderboard's unique_key carries the email, so one person whose address
-- changes within a day keeps both rows through FINAL, and both would reach the
-- join — repeating the session match and reporting a credits_delta twice.
-- Nothing downstream would show that the duplicate was ours rather than the
-- vendor's, which is the one thing a reconciliation view must not do.
leaderboard AS (

    SELECT
        coalesce(tenant_id, '')                     AS insight_tenant_id,
        coalesce(source_id, '')                     AS source_id,
        trim(user_id)                               AS user_id,
        toDateOrNull(date)                          AS day,
        toFloat64OrNull(toString(credits))          AS credits
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_user_daily') }} FINAL
    WHERE user_id IS NOT NULL
      AND trim(user_id) != ''
      AND date IS NOT NULL
    ORDER BY _airbyte_extracted_at DESC
    LIMIT 1 BY coalesce(tenant_id, ''), coalesce(source_id, ''), trim(user_id), toDateOrNull(date)

),

sessions AS (

    SELECT
        coalesce(tenant_id, '')                     AS insight_tenant_id,
        coalesce(source_id, '')                     AS source_id,
        trim(user_id)                               AS user_id,
        toDateOrNull(date)                          AS day,
        toFloat64OrNull(toString(credit_total))     AS credit_total,
        toFloat64OrNull(toString(on_demand_credits)) AS on_demand_credits
    FROM {{ source('bronze_chatgpt_team', 'chatgpt_team_codex_sessions_daily') }} FINAL
    WHERE user_id IS NOT NULL
      AND trim(user_id) != ''
      AND date IS NOT NULL

)

SELECT
    coalesce(l.insight_tenant_id, s.insight_tenant_id)  AS insight_tenant_id,
    coalesce(l.source_id, s.source_id)                  AS source_id,
    coalesce(l.user_id, s.user_id)                      AS user_id,
    coalesce(l.day, s.day)                              AS day,
    multiIf(
        l.day IS NOT NULL AND s.day IS NOT NULL, 'both',
        l.day IS NOT NULL,                       'leaderboard_only',
                                                 'sessions_only'
    )                                                   AS seen_in,
    -- The authoritative total, from the endpoint that can prove a complete
    -- read, beside the same figure from the one that cannot.
    l.credits                                           AS leaderboard_credits,
    s.credit_total                                      AS sessions_credit_total,
    s.on_demand_credits                                 AS sessions_on_demand_credits,
    -- NULL where either side is absent: a missing reading is not a zero
    -- difference, and rendering it as one would hide exactly the rows this
    -- view exists to show.
    if(
        l.day IS NOT NULL AND s.day IS NOT NULL,
        l.credits - s.credit_total,
        CAST(NULL AS Nullable(Float64))
    )                                                   AS credits_delta
FROM leaderboard AS l
FULL OUTER JOIN sessions AS s
             ON l.insight_tenant_id = s.insight_tenant_id
            AND l.source_id = s.source_id
            AND l.user_id = s.user_id
            AND l.day = s.day
