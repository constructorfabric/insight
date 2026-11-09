{% macro jira_link_visibility(reads, candidates) %}
WITH availability_versions AS (
    SELECT
        ifNull(tenant_id, '') AS availability_tenant_id,
        COALESCE(source_id, '') AS availability_source_id,
        COALESCE(jira_id, '') AS availability_issue_id,
        if(availability = 'present', toDateTime64(last_seen_at, 3),
           toDateTime64(last_seen_at, 3)
               + INTERVAL 1 MILLISECOND) AS availability_effective_at,
        argMax(availability, _tracked_at) AS availability_value
    FROM {{ ref('jira__issue_availability_snapshot') }}
    WHERE (ifNull(tenant_id, ''), COALESCE(source_id, ''), COALESCE(jira_id, '')) IN (
        SELECT ifNull(tenant_id, ''), insight_source_id, target_id
        FROM {{ candidates }}
    )
    GROUP BY availability_tenant_id, availability_source_id, availability_issue_id, availability_effective_at
),

project_versions AS (
    SELECT
        ifNull(tenant_id, '') AS visibility_tenant_id,
        COALESCE(source_id, '') AS visibility_source_id,
        COALESCE(project_key, '') AS visibility_project_key,
        if(is_visible = 1, toDateTime64(last_seen_at, 3),
           toDateTime64(last_seen_at, 3)
               + INTERVAL 1 MILLISECOND) AS visibility_effective_at,
        argMax(is_visible, _tracked_at) AS visibility_value
    FROM {{ ref('jira__project_visibility_snapshot') }}
    WHERE (ifNull(tenant_id, ''), COALESCE(source_id, ''), COALESCE(project_key, '')) IN (
        SELECT ifNull(tenant_id, ''), insight_source_id, splitByChar('-', target_key)[1]
        FROM {{ candidates }}
    )
    GROUP BY visibility_tenant_id, visibility_source_id, visibility_project_key, visibility_effective_at
)

SELECT
    r.*,
    if(a.availability_value IS NOT NULL, a.availability_value IN ('present', 'deleted'),
       COALESCE(p.visibility_value, 1) = 1) AS other_issue_visible
FROM {{ reads }} AS r
ASOF LEFT JOIN availability_versions AS a
    ON a.availability_tenant_id = ifNull(r.tenant_id, '')
   AND a.availability_source_id = r.insight_source_id
   AND a.availability_issue_id = r.target_id
   AND COALESCE(r.read_at, toDateTime64(0, 3)) >= a.availability_effective_at
ASOF LEFT JOIN project_versions AS p
    ON p.visibility_tenant_id = ifNull(r.tenant_id, '')
   AND p.visibility_source_id = r.insight_source_id
   AND p.visibility_project_key = splitByChar('-', r.target_key)[1]
   AND COALESCE(r.read_at, toDateTime64(0, 3)) >= p.visibility_effective_at
{% endmacro %}
