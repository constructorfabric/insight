-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_deployment_events']
) }}

-- GitHub deployment statuses -> the deployment event log. Kept as events
-- rather than folded into the deployment: a status can land syncs after its
-- deployment, and an incremental fold would never revisit the parent row.
-- Readers take the latest event per deployment_id (argMax on created_at).
SELECT
    tenant_id,
    source_id,
    unique_key,
    COALESCE(repo_full_name, '') AS repo_full_name,
    toString(COALESCE(deployment_id, 0)) AS deployment_id,
    -- Vendor status id — monotonic per deployment, the tiebreaker readers
    -- fold on when two events share a created_at second.
    COALESCE(id, 0) AS event_id,
    -- GitHub states: success | failure | error | inactive | in_progress |
    -- queued | pending. error is an infrastructure failure of the deploy
    -- itself — a red outcome, kept distinct from failure of the deployed code.
    COALESCE(state, '') AS state,
    COALESCE(environment, '') AS environment,
    COALESCE(creator_login, '') AS creator_login,
    parseDateTimeBestEffortOrNull(created_at) AS created_at,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM {{ source('bronze_github', 'deployment_statuses') }} FINAL
{% if is_incremental() %}
WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
