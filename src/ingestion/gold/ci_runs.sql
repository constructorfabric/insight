{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'entity_id', 'metric_date'],
    partition_by='toYYYYMM(metric_date)',
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings()
) }}

-- One row per decided pipeline run, with its dimensions resolved and its
-- duration expressed in the units the measures read. A semantic-layer dataset:
-- measures count and fold it, drilldown projects it.
--
-- TENANT grain: a pipeline run belongs to the organization, not to a person,
-- so entity_id IS the tenant id and no identity join happens. The actor the
-- source names is deliberately not carried into gold.
--
-- One row per RUN, not per attempt: the source lists only the latest attempt
-- of a retried run, so the latest attempt is the run's state and `attempt > 1`
-- is the retry marker. Undecided (in-flight) runs are excluded — they
-- re-arrive decided on a later sync.
WITH known_commits AS (
    SELECT DISTINCT
        tenant_id,
        commit_hash
    FROM {{ ref('class_git_commits') }} FINAL
    WHERE commit_hash != ''
),

runs AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        pipeline_key,
        pipeline_name,
        run_id,
        run_number,
        attempt,
        is_retry,
        trigger_category,
        outcome,
        is_gate,
        branch,
        started_at,
        duration_s,
        -- Whether the run's head commit was ever collected by the commits
        -- stream. PR runs build synthetic merge refs and fork commits the
        -- stream never sees, so this is the honest joinability ceiling for
        -- any run-to-commit analysis — served as its own measure.
        (tenant_id, commit_sha) IN (
            SELECT tenant_id, commit_hash FROM known_commits
        ) AS commit_known
    FROM {{ ref('class_git_ci_runs') }} FINAL
    WHERE outcome != ''
      AND started_at IS NOT NULL
    ORDER BY attempt DESC
    LIMIT 1 BY tenant_id, source_id, repo_full_name, run_id
)

SELECT
    assumeNotNull(tenant_id)                                     AS tenant_id,
    assumeNotNull(tenant_id)                                     AS entity_id,
    coalesce(toString(source_id), '')                            AS source_id,
    toString(run_id)                                             AS run_id,
    toString(run_number)                                         AS run_number,
    toDate(assumeNotNull(started_at))                            AS metric_date,
    toDateTime64(assumeNotNull(started_at), 3)                   AS started_at,
    toUInt8(is_gate)                                             AS is_gate,
    toUInt8(is_retry)                                            AS is_retry,
    toUInt32(attempt)                                            AS attempt,
    toUInt8(commit_known)                                        AS commit_known,
    branch                                                       AS branch,
    -- Duration in both units the measures serve, so neither measure divides at
    -- read time and a zero-duration run reads as the exclusion its filter
    -- expresses rather than as a converted NULL.
    toFloat64(coalesce(duration_s, 0)) / 60                      AS duration_min,
    toFloat64(coalesce(duration_s, 0)) / 3600                    AS duration_h,
    concat(pipeline_name, ' #', toString(run_number))            AS run_label,
    repo_full_name                                               AS repository_value,
    repo_full_name                                               AS repository_label,
    pipeline_key                                                 AS pipeline_value,
    pipeline_name                                                AS pipeline_label,
    trigger_category                                             AS trigger_value,
    trigger_category                                             AS trigger_label,
    coalesce(outcome, '')                                        AS outcome_value,
    coalesce(outcome, '')                                        AS outcome_label,
    leftPad(toString(intDiv(toHour(started_at), 2) * 2), 2, '0') AS hour_block_value,
    concat(
        leftPad(toString(intDiv(toHour(started_at), 2) * 2), 2, '0'),
        '–',
        leftPad(toString(intDiv(toHour(started_at), 2) * 2 + 2), 2, '0')
    )                                                            AS hour_block_label
FROM runs
WHERE tenant_id IS NOT NULL
