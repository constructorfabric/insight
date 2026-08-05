{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'entity_type', 'cohort_key', 'entity_id'],
    schema=var('gold_database'),
    alias='metric_entity_cohorts_current',
    tags=['gold']
) }}

-- Canonical cohort membership: one row per (tenant, person, cohort_key).
--
-- `entity_id` IS the canonical person id, matching the observation relations,
-- so the peer compiler joins one key and needs no collapsing of its own. HR
-- rows whose email identity cannot resolve are absent — with entity_id being
-- the person id there is no identity to serve them under, and a peer pool is
-- exactly where a guessed identity would do damage.
--
-- CONTESTED MEMBERSHIP: two HR emails of one person can name different org
-- units. Such a person is EXCLUDED (`uniqExact(cohort_id) = 1`) rather than
-- tie-broken — an arbitrary winner would silently compare them against the
-- wrong team's percentiles. The guard lives here, at the grain it applies to,
-- not in every query that reads this view.
--
-- A TABLE, materialized in the same gold build as the observation relations:
-- as a view it resolved against the LIVE identity map on every peer request,
-- so after an identity sync and before the next gold rebuild its entity_id
-- could disagree with the observations' — missing targets and wrong peer
-- membership. One build, one resolution snapshot, everywhere.

-- `resolved_cohort_id`, NOT `cohort_id`: an aggregate alias that shadows the
-- source column makes ClickHouse read it as an aggregate inside an aggregate
-- (ILLEGAL_AGGREGATION, code 184). Renamed back one level out.
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
        {{ canonical_entity_id() }},
        'org_unit' AS cohort_key,
        people.cohort_id AS cohort_id
    FROM (
        SELECT
            workspace_id AS tenant_id,
            lower(assumeNotNull(email)) AS entity_id,
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
            entity_id,
            coalesce(parseDateTimeBestEffortOrNull(toString(valid_from)), toDateTime('1970-01-01')) DESC,
            unique_key DESC
        LIMIT 1 BY tenant_id, entity_id
    ) AS people
    {{ resolved_person_id_join('people') }}
    WHERE people.tenant_id IS NOT NULL
      AND people.tenant_id != ''
      AND people.entity_id IS NOT NULL
      AND people.entity_id != ''
      AND people.cohort_id IS NOT NULL
      AND {{ resolved_only() }}
    ) AS resolved
    GROUP BY tenant_id, entity_type, entity_id, cohort_key
    HAVING uniqExact(cohort_id) = 1
)
