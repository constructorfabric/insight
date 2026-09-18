-- depends_on: {{ ref('gitlab__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['gitlab', 'silver:class_git_item_events']
) }}

-- Merge-request lifecycle events: every state transition (GitLab keeps
-- closed, reopened and merged as resource state events, so a request closed
-- and reopened twice is four events here and one row in pull_requests) and
-- every label change. Neither event reports where it came from; NULL is the
-- honest previous value, not '' claiming it was empty.
WITH projects AS (
    SELECT
        tenant_id,
        source_id,
        id AS project_id,
        COALESCE(namespace_full_path, '') AS project_key,
        COALESCE(path, '') AS repo_slug
    FROM {{ source('bronze_gitlab', 'repositories') }} FINAL
),

state_events AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        project_id,
        COALESCE(mr_iid, 0) AS item_number,
        toString(COALESCE(id, 0)) AS event_id,
        parseDateTimeBestEffortOrNull(created_at) AS event_at,
        COALESCE(user_username, '') AS actor_name,
        'state' AS field_id,
        'set' AS delta_action,
        multiIf(
            state IN ('opened', 'reopened'), 'open',
            COALESCE(state, '')
        ) AS delta_value_id,
        _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'pull_request_state_events') }} FINAL
),

label_events AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        project_id,
        COALESCE(mr_iid, 0) AS item_number,
        toString(COALESCE(id, 0)) AS event_id,
        parseDateTimeBestEffortOrNull(created_at) AS event_at,
        COALESCE(user_username, '') AS actor_name,
        'label' AS field_id,
        multiIf(action = 'remove', 'remove', action = 'add', 'add', '') AS delta_action,
        COALESCE(label_name, '') AS delta_value_id,
        _airbyte_extracted_at
    FROM {{ source('bronze_gitlab', 'pull_request_label_events') }} FINAL
),

events AS (
    SELECT * FROM state_events
    UNION ALL
    SELECT * FROM label_events
)

SELECT
    e.tenant_id AS tenant_id,
    e.source_id AS source_id,
    e.unique_key AS unique_key,
    p.project_key AS project_key,
    p.repo_slug AS repo_slug,
    'pull_request' AS item_type,
    e.item_number AS item_number,
    e.event_id AS event_id,
    e.event_at AS event_at,
    e.actor_name AS actor_name,
    e.field_id AS field_id,
    e.delta_action AS delta_action,
    e.delta_value_id AS delta_value_id,
    '' AS delta_value_display,
    CAST(NULL AS Nullable(String)) AS prev_value_id,
    CAST(NULL AS Nullable(String)) AS prev_value_display,
    'insight_gitlab' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    e._airbyte_extracted_at AS _airbyte_extracted_at
FROM events AS e
INNER JOIN projects AS p
    ON p.tenant_id = e.tenant_id
    AND p.source_id = e.source_id
    AND p.project_id = e.project_id
{% if is_incremental() %}
WHERE e._airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
{% endif %}
