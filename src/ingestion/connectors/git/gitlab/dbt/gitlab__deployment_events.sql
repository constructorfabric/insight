-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_deployment_events']
) }}

-- Every (deployment, status) row bronze holds is one event: GitLab exposes no
-- status history, so the history is the succession of statuses the syncs
-- observed, each stamped with the updated_at that carried it. Readers take
-- the latest event per deployment_id (argMax on created_at, event_id).
SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_path, '') AS repo_full_name,
    toString(COALESCE(id, 0)) AS deployment_id,
    -- Monotonic per deployment: the second the status was set.
    toInt64(COALESCE(toUnixTimestamp(parseDateTimeBestEffortOrNull(updated_at)), 0)) AS event_id,
    -- GitLab statuses mapped onto the class vocabulary shared with the other
    -- deployment sources; canceled and blocked are their own outcomes.
    multiIf(
        status = 'success', 'success',
        status = 'failed', 'failure',
        status = 'running', 'in_progress',
        status = 'created', 'pending',
        status = 'blocked', 'queued',
        status = 'canceled', 'cancelled',
        COALESCE(status, '')
    ) AS state,
    COALESCE(environment_name, '') AS environment,
    COALESCE(user_username, '') AS creator_login,
    parseDateTimeBestEffortOrNull(updated_at) AS created_at,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_gitlab', 'deployments') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
