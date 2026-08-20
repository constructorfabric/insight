{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['source_key'],
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
-- Reads the SAME resolution the build stamped into evidence: entity_id = ''
-- marks a row identity could not resolve, source_entity_id keeps the email
-- behind it. No join here — coverage answers for exactly the snapshot the
-- serving relations were built from, never a fresher map.
--
-- Row-weighted on purpose: a person who produced 500 unresolved commits weighs
-- 500× a person with one — matching what the dashboards actually lose.
-- (`unresolved_people` counts the distinct unknown emails behind it — roughly
-- "how many operator decisions would close the gap".)
--
-- `hr_cohorts` is the peer-comparison membership (one row per person, not
-- activity): unresolved there means the HR email itself is unknown to
-- identity — usually a seeding gap, and it distorts peers for whole teams.
-- The cohort relation drops unresolved rows, so this branch is the one place
-- that still joins the map — inside the same build, so it is the same snapshot.

WITH source_rows AS (
    SELECT source_key, entity_id, source_entity_id
    FROM {{ ref('git_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id, source_entity_id
    FROM {{ ref('ai_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id, source_entity_id
    FROM {{ ref('collab_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id, source_entity_id
    FROM {{ ref('task_metric_evidence') }}

    UNION ALL

    SELECT source_key, entity_id, source_entity_id
    FROM {{ ref('wiki_metric_evidence') }}

    UNION ALL

    -- Seat spend: the one evidence relation whose unresolved rows are money,
    -- so its match rate answers "how much of the billed amount reaches nobody".
    SELECT source_key, entity_id, source_entity_id
    FROM {{ ref('ai_cost_metric_evidence') }}

    UNION ALL

    SELECT
        'hr_cohorts' AS source_key,
        -- Null-proof under EITHER join_use_nulls setting (models differ): the
    -- condition is non-Nullable via coalesce, and person_id is read only on
    -- the matched branch, so entity_id is a plain String fit for the sort key.
    if(
        coalesce(identity_map.email, '') != '',
        toString(assumeNotNull(identity_map.person_id)),
        ''
    ) AS entity_id,
        people.source_entity_id AS source_entity_id
    FROM (
        SELECT lower(assumeNotNull(email)) AS source_entity_id
        FROM {{ ref('class_people') }}
        WHERE email IS NOT NULL
          AND email != ''
    ) AS people
    LEFT JOIN ({{ resolve_person_id() }}) AS identity_map
        ON identity_map.email = people.source_entity_id
)
SELECT
    source_key,
    count() AS observation_rows,
    countIf(entity_id = '') AS unresolved_rows,
    uniqExactIf(source_entity_id, entity_id = '') AS unresolved_people,
    round(100 * countIf(entity_id != '') / count(), 1) AS match_rate_pct
FROM source_rows
GROUP BY source_key
