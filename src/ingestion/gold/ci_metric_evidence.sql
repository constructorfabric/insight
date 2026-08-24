{{ metric_evidence_table() }}

-- CI evidence is TENANT-grain: a pipeline run belongs to the organization,
-- not to a person (issue #2803, decision 5), so there is no identity join —
-- entity_id IS the tenant id, the shape the results compiler expects for
-- entity_type='tenant' (`entity_id = tenant_id`).
--
-- One evidence row per RUN, not per attempt: the source lists only the
-- latest attempt of a retried run, so the latest attempt is the run's state
-- and `attempt > 1` is the retry marker. Undecided (in-flight) runs are
-- excluded — they re-arrive decided on a later sync.
WITH runs AS (
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
        toDate(started_at) AS metric_date
    FROM {{ ref('class_git_ci_runs') }} FINAL
    WHERE outcome != ''
      AND started_at IS NOT NULL
    ORDER BY attempt DESC
    LIMIT 1 BY tenant_id, source_id, repo_full_name, run_id
),

run_rows AS (
    SELECT
        tenant_id,
        metric_date,
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
        CAST(
            [
                tuple('repository', repo_full_name, CAST(NULL AS Nullable(String))),
                tuple('pipeline', pipeline_key, toNullable(pipeline_name)),
                tuple('trigger', trigger_category, CAST(NULL AS Nullable(String))),
                tuple('outcome', outcome, CAST(NULL AS Nullable(String)))
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS run_dimensions
    FROM runs
),

deployment_outcomes AS (
    SELECT
        tenant_id,
        source_id,
        deployment_id,
        -- The vendor status id breaks created_at ties: two events in one
        -- second would otherwise pick a nondeterministic outcome.
        argMax(state, (created_at, event_id)) AS state
    FROM {{ ref('class_git_deployment_events') }} FINAL
    GROUP BY tenant_id, source_id, deployment_id
),

-- Outcome = the latest status event; a deployment with no event yet is
-- 'pending' and stays visible rather than rounded away.
deployment_rows AS (
    SELECT
        d.tenant_id AS tenant_id,
        toDate(d.created_at) AS metric_date,
        d.repo_full_name AS repo_full_name,
        d.deployment_id AS deployment_id,
        d.environment AS environment,
        if(coalesce(o.state, '') = '', 'pending', o.state) AS outcome,
        if(d.is_production = 1, 'production', 'preview') AS env_kind,
        CAST(
            [
                tuple('repository', d.repo_full_name, CAST(NULL AS Nullable(String))),
                tuple('environment', d.environment, CAST(NULL AS Nullable(String))),
                tuple('outcome', if(coalesce(o.state, '') = '', 'pending', o.state), CAST(NULL AS Nullable(String))),
                tuple('env_kind', if(d.is_production = 1, 'production', 'preview'), CAST(NULL AS Nullable(String)))
            ] AS Array(Tuple(key String, value String, label Nullable(String)))
        ) AS deployment_dimensions
    FROM (
        SELECT
            tenant_id,
            source_id,
            repo_full_name,
            deployment_id,
            environment,
            is_production,
            created_at
        FROM {{ ref('class_git_deployments') }} FINAL
        WHERE created_at IS NOT NULL
    ) AS d
    LEFT JOIN deployment_outcomes AS o
        ON d.tenant_id = o.tenant_id
       AND d.source_id = o.source_id
       AND d.deployment_id = o.deployment_id
)

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'ci' AS source_key,
    'tenant' AS entity_type,
    assumeNotNull(tenant_id) AS entity_id,
    assumeNotNull(tenant_id) AS source_entity_id,
    assumeNotNull(metric_date) AS metric_date,
    toNullable(toDateTime64(started_at, 3)) AS observed_at,
    measure.1 AS measure_key,
    concat(repo_full_name, ':', toString(run_id), ':', measure.1) AS record_id,
    'ci_run' AS record_kind,
    'event' AS granularity,
    concat(pipeline_name, ' #', toString(run_number)) AS record_label,
    toNullable(measure.2) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    run_dimensions AS dimensions,
    map(
        'repository', repo_full_name,
        'pipeline', pipeline_key,
        'branch', branch,
        'outcome', outcome,
        'attempt', toString(attempt)
    ) AS details
FROM run_rows
-- One row per measure the run feeds; a NULL slot means it does not feed that
-- measure and the row is dropped below.
ARRAY JOIN
    arrayFilter(
        m -> m.2 IS NOT NULL,
        [
            tuple('runs', toNullable(toFloat64(1))),
            tuple('gate_runs', if(is_gate = 1, toNullable(toFloat64(1)), NULL)),
            tuple('gate_passed', if(is_gate = 1 AND outcome = 'success', toNullable(toFloat64(1)), NULL)),
            tuple('gate_first_try_passed', if(is_gate = 1 AND outcome = 'success' AND attempt = 1, toNullable(toFloat64(1)), NULL)),
            tuple('gate_retried', if(is_gate = 1 AND is_retry = 1, toNullable(toFloat64(1)), NULL)),
            tuple('run_duration_min', if(is_gate = 1 AND coalesce(duration_s, 0) > 0, toNullable(toFloat64(duration_s) / 60), NULL)),
            tuple('run_hours', if(coalesce(duration_s, 0) > 0, toNullable(toFloat64(duration_s) / 3600), NULL))
        ]
    ) AS measure
WHERE tenant_id IS NOT NULL

UNION ALL

SELECT
    assumeNotNull(tenant_id) AS tenant_id,
    'ci' AS source_key,
    'tenant' AS entity_type,
    assumeNotNull(tenant_id) AS entity_id,
    assumeNotNull(tenant_id) AS source_entity_id,
    assumeNotNull(metric_date) AS metric_date,
    CAST(NULL AS Nullable(DateTime64(3))) AS observed_at,
    'deployments' AS measure_key,
    concat(repo_full_name, ':', deployment_id, ':deployments') AS record_id,
    'deployment' AS record_kind,
    'event' AS granularity,
    concat(environment, ' · ', outcome) AS record_label,
    toNullable(toFloat64(1)) AS contribution,
    CAST(NULL AS Nullable(String)) AS subject_key,
    deployment_dimensions AS dimensions,
    map(
        'repository', repo_full_name,
        'environment', environment,
        'outcome', outcome,
        'env_kind', env_kind
    ) AS details
FROM deployment_rows
WHERE tenant_id IS NOT NULL
