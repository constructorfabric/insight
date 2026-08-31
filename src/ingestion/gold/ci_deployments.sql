{{ config(
    materialized='table',
    engine='MergeTree',
    order_by=['tenant_id', 'entity_id', 'metric_date'],
    partition_by='toYYYYMM(metric_date)',
    schema=var('gold_database'),
    tags=['gold'],
    query_settings=metric_serving_query_settings(join_use_nulls=1)
) }}

-- One row per deployment, with the latest status event folded in as its
-- outcome. A semantic-layer dataset: measures count it, drilldown projects it.
--
-- TENANT grain: a deployment belongs to the organization, so entity_id IS the
-- tenant id and no identity join happens. The creator the source names is
-- deliberately not carried into gold.
--
-- A deployment with no status event yet is 'pending' and stays visible rather
-- than rounded away.
WITH deployment_outcomes AS (
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

deployments AS (
    SELECT
        tenant_id,
        source_id,
        repo_full_name,
        deployment_id,
        environment,
        is_production,
        is_transient,
        created_at
    FROM {{ ref('class_git_deployments') }} FINAL
    WHERE created_at IS NOT NULL
),

resolved AS (
    SELECT
        assumeNotNull(d.tenant_id)                         AS tenant_id,
        coalesce(toString(d.source_id), '')                AS source_id,
        d.deployment_id                                    AS deployment_id,
        d.repo_full_name                                   AS repo_full_name,
        d.environment                                      AS environment,
        assumeNotNull(d.created_at)                        AS created_at,
        -- coalesce against a literal so the outer relation carries a plain
        -- String: `join_use_nulls` makes every column of the right side
        -- nullable, and a nullable dimension groups a missing outcome under
        -- NULL instead of under the visible 'pending'.
        coalesce(nullIf(o.state, ''), 'pending')           AS outcome,
        -- Non-production splits by the vendor's transient flag: 'preview' is
        -- an environment that exists to be torn down, 'static' one that
        -- persists (staging, QA).
        multiIf(
            d.is_production = 1, 'production',
            d.is_transient = 1, 'preview',
            'static'
        )                                                  AS env_kind
    FROM deployments AS d
    LEFT JOIN deployment_outcomes AS o
        ON d.tenant_id = o.tenant_id
       AND d.source_id = o.source_id
       AND d.deployment_id = o.deployment_id
    WHERE d.tenant_id IS NOT NULL
)

SELECT
    tenant_id                                AS tenant_id,
    tenant_id                                AS entity_id,
    source_id                                AS source_id,
    deployment_id                            AS deployment_id,
    toDate(created_at)                       AS metric_date,
    toDateTime64(created_at, 3)              AS created_at,
    concat(environment, ' · ', outcome)      AS deployment_label,
    repo_full_name                           AS repository_value,
    repo_full_name                           AS repository_label,
    environment                              AS environment_value,
    environment                              AS environment_label,
    outcome                                  AS outcome_value,
    outcome                                  AS outcome_label,
    env_kind                                 AS env_kind_value,
    env_kind                                 AS env_kind_label
FROM resolved
