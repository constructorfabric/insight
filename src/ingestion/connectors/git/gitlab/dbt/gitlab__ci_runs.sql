-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_ci_runs']
) }}

-- GitLab pipelines -> the vendor-neutral CI-run class.
--
-- A project has one pipeline definition (.gitlab-ci.yml), so the project path
-- is the pipeline key; GitLab gives the pipeline no name of its own on the
-- versions this query supports. A retry re-runs jobs inside the same
-- pipeline rather than minting an attempt, so every row is attempt 1.
--
-- The gate (is_gate) is the class's single definition of "a run that counts
-- toward pass rates": commit-triggered AND carrying a decided outcome.
-- canceled, skipped and manual (waiting at a manual job) are undecided.
--
-- FINAL: a pipeline is mutable while it runs (status and updated_at move), so
-- a window re-fetch within one sync can leave a pre-merge duplicate in bronze.
SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_path, '') AS repo_full_name,
    COALESCE(repo_path, '') AS pipeline_key,
    '' AS pipeline_name,
    COALESCE(id, 0) AS run_id,
    COALESCE(iid, 0) AS run_number,
    toInt64(1) AS attempt,
    0 AS is_retry,
    multiIf(
        source = 'push', 'push',
        source IN ('merge_request_event', 'external_pull_request_event'), 'pull_request',
        source = 'schedule', 'schedule',
        source = 'web', 'manual',
        'other'
    ) AS trigger_category,
    COALESCE(source, '') AS trigger_raw,
    -- Class outcome vocabulary; '' = still undecided.
    multiIf(
        status = 'success', 'success',
        status = 'failed', 'failure',
        status = 'canceled', 'cancelled',
        status = 'skipped', 'skipped',
        ''
    ) AS outcome,
    if(
        source IN ('push', 'merge_request_event', 'external_pull_request_event')
        AND status IN ('success', 'failed'),
        1, 0
    ) AS is_gate,
    COALESCE(ref, '') AS branch,
    COALESCE(sha, '') AS commit_sha,
    COALESCE(user_username, '') AS actor_login,
    parseDateTimeBestEffortOrNull(created_at) AS created_at,
    parseDateTimeBestEffortOrNull(started_at) AS started_at,
    parseDateTimeBestEffortOrNull(finished_at) AS finished_at,
    -- GitLab's own duration: wall-clock seconds the pipeline ran, excluding
    -- queue time, NULL until it finishes.
    if(status IN ('success', 'failed', 'canceled'), toInt64(duration), CAST(NULL AS Nullable(Int64))) AS duration_s,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'pipelines') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
