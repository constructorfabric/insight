{{ config(
    materialized='view',
    schema=var('gold_database'),
    alias='identity_resolution_coverage',
    tags=['gold']
) }}

-- Identity-resolution match rate, measured over ACTIVITY (source rows), not
-- over aliases: one row per source with how much of its recorded work resolves
-- to a canonical person. This is the measuring device for the "match rate is
-- reported" outcome of the identity epic and the prioritization signal for
-- resolution-quality work — a source with a low rate is where identity is
-- missing that source's emails.
--
-- Left-joins the same `person_map` the runtime joins, and stays a view, so the
-- rate describes what a dashboard shows now rather than what a build stamped.
--
-- Row-weighted on purpose: a person who produced 500 unresolved commits weighs
-- 500× a person with one — matching what the dashboards actually lose.
-- (`unresolved_people` counts the distinct unknown emails behind it — roughly
-- "how many operator decisions would close the gap".)
--
-- `hr_cohorts` is the peer-comparison membership (one row per person, not
-- activity): unresolved there means the HR email itself is unknown to
-- identity — usually a seeding gap, and it distorts peers for whole teams.

WITH source_emails AS (
    SELECT source_key, entity_id AS email
    FROM {{ ref('git_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id AS email
    FROM {{ ref('ai_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id AS email
    FROM {{ ref('collab_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id AS email
    FROM {{ ref('task_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id AS email
    FROM {{ ref('wiki_metric_evidence') }}

    UNION ALL

    -- Seat spend: the one evidence relation whose unresolved rows are money,
    -- so its match rate answers "how much of the billed amount reaches nobody".
    SELECT source_key, entity_id AS email
    FROM {{ ref('ai_cost_metric_evidence') }}

    UNION ALL

    -- DISTINCT is the read-time dedup for this RMT read: the branch counts
    -- each HR email once, so collapsing row versions and duplicates is the
    -- same operation here.
    SELECT DISTINCT
        'hr_cohorts' AS source_key,
        {{ normalized_email('assumeNotNull(email)') }} AS email
    FROM {{ ref('class_people') }} FINAL
    WHERE email IS NOT NULL
      AND email != ''
),
resolution AS (
    SELECT
        source_emails.source_key AS source_key,
        source_emails.email AS email,
        -- LEFT: an unresolved row is the thing being counted, so it must
        -- survive the join. `resolved` is non-Nullable under either
        -- join_use_nulls setting.
        coalesce(person_map.email, '') != '' AS resolved
    FROM source_emails
    LEFT JOIN {{ ref('person_map') }} AS person_map
        ON person_map.email = source_emails.email
)
SELECT
    source_key,
    count() AS observation_rows,
    countIf(NOT resolved) AS unresolved_rows,
    uniqExactIf(email, NOT resolved) AS unresolved_people,
    round(100 * countIf(resolved) / count(), 1) AS match_rate_pct
FROM resolution
GROUP BY source_key
