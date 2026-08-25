-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'staging'],
    query_settings={'join_use_nulls': 1}
) }}

-- Current availability per ever-seen issue (specs/DELETION-AND-VISIBILITY.md).
-- An issue is absent when its last census observation is older than the
-- census high-water mark minus a one-sync tolerance. Absence is then
-- classified against the project visibility roster of the same generation:
--
--   present     seen in the latest census generation
--   deleted     absent, project live and visible, below the mass threshold
--   unobserved  absent, project live, but >= threshold of the project's known
--               issues vanished at once — a partial sync or permission edge,
--               not a plausible mass deletion; reclassifies on a later run
--   archived    absent, project archived
--   trashed     absent, project in Jira's project trash
--   access_lost absent, project no longer visible to the account at all
--
-- Issues known only from bronze_jira.jira_issue (never censused) enter the
-- universe with an epoch last-seen: absent from the very first census means
-- deleted before the mechanism landed — the intended historical backfill.

WITH census AS (
    SELECT
        unique_key,
        tenant_id,
        source_id,
        jira_id,
        project_key,
        CAST(NULL AS Nullable(String))                       AS id_readable,
        _airbyte_extracted_at                                AS seen_at
    FROM {{ source('bronze_jira', 'jira_issue_census') }} FINAL
),

known AS (
    SELECT
        unique_key,
        tenant_id,
        source_id,
        jira_id,
        project_key,
        id_readable,
        seen_at
    FROM census

    UNION ALL

    -- COALESCE keeps the key well-defined when a stamp column is NULL;
    -- otherwise every unstamped row would collapse into one NULL-key group.
    SELECT
        concat(COALESCE(tenant_id, ''), '-', COALESCE(source_id, ''), '-',
               COALESCE(jira_id, ''))                        AS unique_key,
        tenant_id,
        source_id,
        jira_id,
        project_key,
        id_readable,
        toDateTime64(0, 3)                                   AS seen_at
    FROM {{ source('bronze_jira', 'jira_issue') }} FINAL
    WHERE jira_id IS NOT NULL

    UNION ALL

    -- Jira project keys cannot contain '-', so the key prefix is the project.
    SELECT
        concat(COALESCE(tenant_id, ''), '-', COALESCE(source_id, ''), '-',
               COALESCE(jira_id, ''))                        AS unique_key,
        tenant_id,
        source_id,
        jira_id,
        splitByChar('-', assumeNotNull(id_readable))[1]      AS project_key,
        id_readable,
        toDateTime64(0, 3)                                   AS seen_at
    FROM {{ source('bronze_jira', 'jira_issue_keys') }} FINAL
    WHERE jira_id IS NOT NULL AND id_readable IS NOT NULL
),

issues AS (
    SELECT
        unique_key,
        any(tenant_id)                                       AS tenant_id,
        any(source_id)                                       AS source_id,
        any(jira_id)                                         AS jira_id,
        argMax(project_key, seen_at)                         AS project_key,
        any(id_readable)                                     AS id_readable,
        max(seen_at)                                         AS last_seen_at
    FROM known
    GROUP BY unique_key
),

generation AS (
    SELECT
        tenant_id,
        source_id,
        max(seen_at)
            - INTERVAL {{ var('jira_census_tolerance_hours', 12) }} HOUR
                                                             AS observed_after
    FROM census
    GROUP BY tenant_id, source_id
),

observed AS (
    SELECT
        i.unique_key                                         AS unique_key,
        i.tenant_id                                          AS tenant_id,
        i.source_id                                          AS source_id,
        i.jira_id                                            AS jira_id,
        i.project_key                                        AS project_key,
        i.id_readable                                        AS id_readable,
        i.last_seen_at                                       AS last_seen_at,
        g.observed_after IS NOT NULL                         AS has_census,
        i.last_seen_at >= g.observed_after                   AS is_present
    FROM issues AS i
    LEFT JOIN generation AS g
        ON g.tenant_id = i.tenant_id AND g.source_id = i.source_id
),

project_absence AS (
    SELECT
        tenant_id,
        source_id,
        project_key,
        avg(NOT is_present)                                  AS absent_share
    FROM observed
    GROUP BY tenant_id, source_id, project_key
)

SELECT
    o.unique_key                                             AS unique_key,
    o.tenant_id                                              AS tenant_id,
    o.source_id                                              AS source_id,
    o.jira_id                                                AS jira_id,
    o.project_key                                            AS project_key,
    o.id_readable                                            AS id_readable,
    multiIf(
        NOT o.has_census,                       'present',
        o.is_present,                           'present',
        p.project_key IS NULL
            OR ifNull(p.is_visible, 0) = 0,     'access_lost',
        p.project_status = 'archived',          'archived',
        p.project_status = 'deleted',           'trashed',
        pa.absent_share >=
            {{ var('jira_availability_mass_threshold', 0.5) }},
                                                'unobserved',
                                                'deleted'
    )                                                        AS availability,
    o.last_seen_at                                           AS last_seen_at
FROM observed AS o
LEFT JOIN {{ ref('jira__project_visibility_state') }} AS p FINAL
    ON p.tenant_id = o.tenant_id
    AND p.source_id = o.source_id
    AND p.project_key = o.project_key
LEFT JOIN project_absence AS pa
    ON pa.tenant_id = o.tenant_id
    AND pa.source_id = o.source_id
    AND pa.project_key = o.project_key
