-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_pull_requests']
) }}

SELECT
    pr.tenant_id,
    pr.source_id,
    pr.unique_key,
    COALESCE(pr.workspace, '') AS project_key,
    COALESCE(pr.repo_slug, '') AS repo_slug,
    COALESCE(pr.id, 0) AS pr_id,
    COALESCE(pr.id, 0) AS pr_number,
    COALESCE(pr.title, '') AS title,
    COALESCE(pr.description, '') AS description,
    multiIf(
        pr.state = 'SUPERSEDED', 'DECLINED',
        COALESCE(pr.state, '')
    ) AS state,
    COALESCE(pr.author_display_name, '') AS author_name,
    '' AS author_email,
    COALESCE(pr.source_branch, '') AS source_branch,
    COALESCE(pr.destination_branch, '') AS destination_branch,
    parseDateTimeBestEffortOrNull(pr.created_on) AS created_on,
    parseDateTimeBestEffortOrNull(pr.updated_on) AS updated_on,
    parseDateTimeBestEffortOrNull(if(pr.state IN ('MERGED', 'DECLINED', 'SUPERSEDED'), toString(pr.updated_on), '')) AS closed_on,
    COALESCE(pr.merge_commit_hash, '') AS merge_commit_hash,
    COALESCE(fc.files_changed, 0) AS files_changed,
    COALESCE(fc.lines_added, 0) AS lines_added,
    COALESCE(fc.lines_removed, 0) AS lines_removed,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    pr._airbyte_extracted_at
FROM {{ source('bronze_bitbucket_cloud', 'pull_requests') }} AS pr
LEFT JOIN (
    SELECT
        prc.tenant_id,
        prc.workspace,
        prc.repo_slug,
        prc.pr_id,
        count() AS files_changed,
        SUM(COALESCE(fc_raw.additions, 0)) AS lines_added,
        SUM(COALESCE(fc_raw.deletions, 0)) AS lines_removed
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_commits') }} AS prc
    INNER JOIN {{ source('bronze_bitbucket_cloud', 'file_changes') }} AS fc_raw
        ON fc_raw.sha = prc.hash
        AND fc_raw.tenant_id = prc.tenant_id
        AND fc_raw.workspace = prc.workspace
        AND fc_raw.repo_slug = prc.repo_slug
    GROUP BY prc.tenant_id, prc.workspace, prc.repo_slug, prc.pr_id
) AS fc
    ON fc.pr_id = pr.id
    AND fc.tenant_id = pr.tenant_id
    AND fc.workspace = pr.workspace
    AND fc.repo_slug = pr.repo_slug
{% if is_incremental() %}
WHERE pr._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
