{{ config(
    materialized='view',
    schema=var('gold_database'),
    alias='metric_entity_cohorts_current',
    tags=['gold']
) }}

-- Canonical cohort membership: one row per (tenant, person, cohort_key).
--
-- `entity_id` IS the canonical person id, because a cohort is a set of people,
-- not of accounts: two HR records of one person must not put them in a peer
-- pool twice, and a pool size that counted accounts would open the disclosure
-- guard on fewer people than it claims.
--
-- SAFETY: a person whose HR records name different org units is EXCLUDED
-- (`uniqExact(cohort_id) = 1`), not tie-broken — an arbitrary winner compares
-- them against the wrong team's percentiles.
--
-- INVARIANT: a view over the live map, resolving in the same query the
-- observations do, so both sides of a peer comparison always agree.
--
-- WORKAROUND: `resolved_cohort_id`, not `cohort_id` — an aggregate alias
-- shadowing the source column reads as an aggregate inside an aggregate
-- (ILLEGAL_AGGREGATION, code 184).
SELECT
    tenant_id,
    entity_type,
    entity_id,
    cohort_key,
    resolved_cohort_id AS cohort_id
FROM (
    SELECT
        tenant_id,
        entity_type,
        entity_id,
        cohort_key,
        any(cohort_id) AS resolved_cohort_id
    FROM (
    SELECT
        assumeNotNull(people.tenant_id) AS tenant_id,
        'person' AS entity_type,
        toString(person_map.person_id) AS entity_id,
        'org_unit' AS cohort_key,
        people.cohort_id AS cohort_id
    FROM (
        SELECT
            workspace_id AS tenant_id,
            {{ normalized_email('assumeNotNull(email)') }} AS email,
            -- The org cohort is keyed by department NAME, matching what the rest of
            -- the serving path calls `org_unit_id` (insight.people projects
            -- `argMax(department)` under that name, and the frontend round-trips the
            -- value back as `org_unit_id in ('Engineering', …)`). This used to
            -- coalesce a `class_people.org_unit_id Nullable(UUID)` column ahead of
            -- the name; that column was always NULL (no `org_units` table exists) and
            -- has been dropped. Do NOT reintroduce a UUID branch here without
            -- migrating insight.people and the frontend in the same change — a
            -- coalesce that prefers UUIDs would emit cohort ids the frontend never
            -- sends, silently emptying every peer metric.
            nullIf(department_name, '') AS cohort_id
        FROM {{ ref('class_people') }}
        WHERE email IS NOT NULL
          AND email != ''
          AND workspace_id IS NOT NULL
          AND workspace_id != ''
        ORDER BY
            tenant_id,
            email,
            coalesce(parseDateTimeBestEffortOrNull(toString(valid_from)), toDateTime('1970-01-01')) DESC,
            unique_key DESC
        LIMIT 1 BY tenant_id, email
    ) AS people
    -- INNER: an HR record whose email identity cannot resolve has no person to
    -- be a peer of, and a peer pool is exactly where a guessed identity would
    -- do damage.
    INNER JOIN {{ ref('person_map') }} AS person_map
        ON person_map.email = people.email
    WHERE people.tenant_id IS NOT NULL
      AND people.tenant_id != ''
      AND people.email != ''
      AND people.cohort_id IS NOT NULL
    ) AS resolved
    GROUP BY tenant_id, entity_type, entity_id, cohort_key
    HAVING uniqExact(cohort_id) = 1
)
