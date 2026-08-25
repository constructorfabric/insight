-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    engine='ReplacingMergeTree',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['jira', 'staging']
) }}

-- Current visibility per ever-seen project (specs/DELETION-AND-VISIBILITY.md).
-- The census stream is full-refresh, so after RMT promotion each project's
-- _airbyte_extracted_at is its last observation; a project absent from the
-- latest census generation (watermark minus a one-sync tolerance) is no
-- longer visible to the service account.

WITH roster AS (
    SELECT
        unique_key,
        tenant_id,
        source_id,
        toString(toInt64(project_id))                       AS project_id,
        project_key,
        project_status,
        _airbyte_extracted_at                               AS seen_at
    FROM {{ source('bronze_jira', 'jira_project_visibility') }} FINAL
),

-- Per (tenant, source): each source instance censuses on its own clock, and
-- one instance's fresher watermark must not mark another's projects stale.
generation AS (
    SELECT
        tenant_id,
        source_id,
        max(seen_at) AS watermark
    FROM roster
    GROUP BY tenant_id, source_id
)

SELECT
    r.unique_key                                            AS unique_key,
    r.tenant_id                                             AS tenant_id,
    r.source_id                                             AS source_id,
    r.project_id                                            AS project_id,
    r.project_key                                           AS project_key,
    toUInt8(r.seen_at >= g.watermark
        - INTERVAL {{ var('jira_census_tolerance_hours', 12) }} HOUR) AS is_visible,
    r.project_status                                        AS project_status,
    r.seen_at                                               AS last_seen_at
FROM roster AS r
INNER JOIN generation AS g
    ON g.tenant_id = r.tenant_id
    AND g.source_id = r.source_id
