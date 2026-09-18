-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_pull_requests_reviewers']
) }}

-- Two sources, two facts. The merge request's `reviewers` array is who was
-- ASKED; the system notes are what they DID — GitLab records every approval,
-- its withdrawal and a request for changes as a system note with the actor
-- and the instant, which the approvals endpoint (current approvers, no time)
-- cannot give. A reviewer who never acted stays a `requested` row.
WITH projects AS (
    SELECT
        tenant_id,
        source_id,
        id AS project_id,
        COALESCE(namespace_full_path, '') AS project_key,
        COALESCE(path, '') AS repo_slug
    FROM {{ source('bronze_gitlab', 'repositories') }} FINAL
),

requested AS (
    SELECT
        mr.tenant_id AS tenant_id,
        mr.source_id AS source_id,
        concat(mr.unique_key, ':reviewer:', toString(JSONExtractInt(reviewer, 'id'))) AS unique_key,
        mr.project_id AS project_id,
        COALESCE(mr.iid, 0) AS pr_id,
        JSONExtractString(reviewer, 'username') AS reviewer_name,
        toString(JSONExtractInt(reviewer, 'id')) AS reviewer_uuid,
        'requested' AS status,
        0 AS approved,
        CAST(NULL AS Nullable(DateTime)) AS reviewed_at,
        mr._airbyte_extracted_at AS _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'pull_requests') }} AS mr FINAL
    ARRAY JOIN JSONExtractArrayRaw(COALESCE(mr.reviewers, '[]')) AS reviewer
),

verdicts AS (
    SELECT
        n.tenant_id AS tenant_id,
        n.source_id AS source_id,
        n.unique_key AS unique_key,
        n.project_id AS project_id,
        COALESCE(n.mr_iid, 0) AS pr_id,
        COALESCE(n.author_username, '') AS reviewer_name,
        toString(COALESCE(n.author_id, 0)) AS reviewer_uuid,
        multiIf(
            n.body = 'approved this merge request', 'approved',
            n.body = 'unapproved this merge request', 'unapproved',
            'changes_requested'
        ) AS status,
        if(n.body = 'approved this merge request', 1, 0) AS approved,
        -- INVARIANT: a withdrawal carries no instant; gold dates a request's
        -- first review by min(reviewed_at), which must never land on one.
        if(n.body = 'unapproved this merge request', CAST(NULL AS Nullable(DateTime)), parseDateTimeBestEffortOrNull(n.created_at)) AS reviewed_at,
        n._airbyte_extracted_at AS _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'pull_request_notes') }} AS n FINAL
    WHERE COALESCE(n.system, false)
      AND (
          n.body IN ('approved this merge request', 'unapproved this merge request')
          OR n.body LIKE 'requested changes%'
      )
),

reviews AS (
    SELECT * FROM requested
    UNION ALL
    SELECT * FROM verdicts
)

SELECT
    r.tenant_id AS tenant_id,
    r.source_id AS source_id,
    r.unique_key AS unique_key,
    p.project_key AS project_key,
    p.repo_slug AS repo_slug,
    r.pr_id AS pr_id,
    r.reviewer_name AS reviewer_name,
    r.reviewer_uuid AS reviewer_uuid,
    r.status AS status,
    r.approved AS approved,
    r.reviewed_at AS reviewed_at,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    r._airbyte_extracted_at AS _airbyte_extracted_at
FROM reviews AS r
INNER JOIN projects AS p
    ON p.tenant_id = r.tenant_id
    AND p.source_id = r.source_id
    AND p.project_id = r.project_id
{% if is_incremental() %}
WHERE r._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
