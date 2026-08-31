{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'entity_id', 'metric_date'],
    partition_by='toYYYYMM(metric_date)',
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- One row per collected commit, dated at its commit time — the denominator of
-- the run-to-commit join-coverage reading: set beside runs and the runs whose
-- head commit was collected, it shows how far the two streams overlap, and so
-- how much any commit-joined CI reading can be trusted.
--
-- TENANT grain: this counts what the connector collected, not what anyone
-- wrote, so entity_id IS the tenant id. The commit's author is deliberately
-- not carried here — git_commits serves the person-grain reading.
SELECT
    assumeNotNull(tenant_id)                                   AS tenant_id,
    assumeNotNull(tenant_id)                                   AS entity_id,
    coalesce(source_id, '')                                    AS source_id,
    project_key                                                AS project_key,
    repo_slug                                                  AS repo_slug,
    commit_hash                                                AS commit_hash,
    toDate(assumeNotNull(date))                                AS metric_date,
    toDateTime64(assumeNotNull(date), 3)                       AS committed_at,
    substring(commit_hash, 1, 10)                              AS commit_reference,
    concat(project_key, '/', repo_slug)                        AS repository_value,
    concat(project_key, '/', repo_slug)                        AS repository_label
FROM {{ ref('class_git_commits') }} FINAL
WHERE tenant_id IS NOT NULL
  AND commit_hash != ''
  AND date IS NOT NULL
-- One row per commit within the repository that collected it: a commit reached
-- through two branches is one commit, and counting it twice would overstate
-- the coverage denominator.
LIMIT 1 BY tenant_id, source_id, project_key, repo_slug, commit_hash
