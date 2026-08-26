-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_ci_runs']
) }}

-- GitHub Actions workflow runs -> the vendor-neutral CI-run class.
--
-- Duration is finished_at - started_at, never created_at: on a re-run
-- created_at stays with the FIRST attempt while run_started_at moves to the
-- last, so mixing them charges the retried run for the wait between attempts.
--
-- The gate (is_gate) is the class's single definition of "a run that counts
-- toward pass rates": commit-triggered AND carrying a decided outcome.
-- action_required is NOT decided — such a run never executed, it stopped at
-- an approval wall with zero duration. cancelled/skipped are undecided too.
-- A metric wanting a different population gets a different class, not a knob
-- here.
--
-- FINAL: a run is mutable while in progress (status/conclusion/updated_at
-- move), so a window re-fetch within one sync can leave a pre-merge duplicate
-- in bronze.
SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_full_name, '') AS repo_full_name,
    -- The workflow FILE identifies the pipeline; the display name is not
    -- unique and can be edited.
    COALESCE(workflow_path, '') AS pipeline_key,
    COALESCE(name, '') AS pipeline_name,
    COALESCE(id, 0) AS run_id,
    COALESCE(run_number, 0) AS run_number,
    COALESCE(run_attempt, 1) AS attempt,
    if(COALESCE(run_attempt, 1) > 1, 1, 0) AS is_retry,
    multiIf(
        event = 'push', 'push',
        event IN ('pull_request', 'pull_request_target'), 'pull_request',
        event = 'merge_group', 'merge_queue',
        event = 'schedule', 'schedule',
        event = 'workflow_dispatch', 'manual',
        'other'
    ) AS trigger_category,
    COALESCE(event, '') AS trigger_raw,
    -- Class outcome vocabulary; '' = still undecided (in progress or queued).
    multiIf(
        conclusion IS NULL, '',
        conclusion IN ('failure', 'startup_failure'), 'failure',
        conclusion = 'stale', 'cancelled',
        conclusion
    ) AS outcome,
    -- Rows are an append archive: runs classified before merge_group joined
    -- the gate keep trigger_category='other' and is_gate=0; only runs
    -- classified from then on carry the merge_queue category.
    if(
        event IN ('push', 'pull_request', 'pull_request_target', 'merge_group')
        AND conclusion IN ('success', 'failure', 'startup_failure', 'timed_out'),
        1, 0
    ) AS is_gate,
    COALESCE(head_branch, '') AS branch,
    COALESCE(head_sha, '') AS commit_sha,
    COALESCE(actor_login, '') AS actor_login,
    parseDateTimeBestEffortOrNull(created_at) AS created_at,
    parseDateTimeBestEffortOrNull(run_started_at) AS started_at,
    parseDateTimeBestEffortOrNull(updated_at) AS finished_at,
    -- NULL rather than a lie when the run is still moving or a timestamp
    -- regressed; zero-duration rows stay (they are real instant failures) and
    -- duration measures exclude them downstream.
    if(
        parseDateTimeBestEffortOrNull(updated_at) >= parseDateTimeBestEffortOrNull(run_started_at)
        AND conclusion IS NOT NULL,
        toNullable(toInt64(dateDiff(
            'second',
            parseDateTimeBestEffortOrNull(run_started_at),
            parseDateTimeBestEffortOrNull(updated_at)
        ))),
        CAST(NULL AS Nullable(Int64))
    ) AS duration_s,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'workflow_runs') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
