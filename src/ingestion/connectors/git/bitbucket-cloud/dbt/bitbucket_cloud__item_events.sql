-- depends_on: {{ ref('bitbucket_cloud__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['bitbucket-cloud', 'silver:class_git_item_events']
) }}

-- Bitbucket reports a pull request's history as activity entries of three
-- kinds — approval, changes_requested, update — and only `update` says what
-- changed: `changes` is a per-field `{"status": {"old": …, "new": …}}` map.
-- This maps that vocabulary onto the class's (field_id, delta_action, value)
-- triple.
--
-- Bitbucket has no issue tracker in this connector's scope, so every row is a
-- pull request; `item_type` is constant rather than derived.
--
-- An approval carries no field of its own — the approver IS the value — so it
-- becomes a reviewer add, mirroring how the GitHub model treats a review
-- request. Only `update` events with a status change report a previous value.
--
-- FINAL: an activity entry is immutable, but the names projected onto it are
-- not — a window re-fetch after a display-name change re-emits the same key
-- with different values.
WITH activity AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        repo_full_name,
        pr_id,
        kind,
        event_date,
        actor_display_name,
        actor_account_id,
        update_state,
        changes,
        _airbyte_extracted_at
    FROM {{ source('bronze_bitbucket_cloud', 'pull_request_activity') }} FINAL
    {% if is_incremental() %}
    WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
    {% endif %}
),

-- `changes` is a JSON object keyed by field name. Only the status transition
-- is a lifecycle fact; the rest (title, description, reviewers) restate what
-- the pull request row already carries.
status_change AS (
    SELECT
        *,
        JSONExtractString(COALESCE(changes, '{}'), 'status', 'old') AS status_old,
        JSONExtractString(COALESCE(changes, '{}'), 'status', 'new') AS status_new
    FROM activity
)

SELECT
    tenant_id,
    source_id,
    unique_key,
    splitByChar('/', COALESCE(repo_full_name, ''))[1] AS project_key,
    splitByChar('/', COALESCE(repo_full_name, ''))[2] AS repo_slug,
    'pull_request' AS item_type,
    COALESCE(pr_id, 0) AS item_number,
    COALESCE(unique_key, '') AS event_id,
    parseDateTimeBestEffortOrNull(event_date) AS event_at,
    COALESCE(actor_display_name, '') AS actor_name,
    multiIf(
        kind IN ('approval', 'changes_requested'), 'reviewer',
        status_new != '' OR COALESCE(update_state, '') != '', 'state',
        ''
    ) AS field_id,
    multiIf(
        kind = 'approval', 'add',
        kind = 'changes_requested', 'remove',
        'set'
    ) AS delta_action,
    multiIf(
        kind IN ('approval', 'changes_requested'), COALESCE(actor_account_id, ''),
        status_new != '', status_new,
        COALESCE(update_state, '')
    ) AS delta_value_id,
    -- The approver's name reads where the account id does not; nothing else
    -- carries a display distinct from the value itself.
    multiIf(
        kind IN ('approval', 'changes_requested'), COALESCE(actor_display_name, ''),
        ''
    ) AS delta_value_display,
    -- Only a status transition reports where it came from; NULL everywhere
    -- else is the honest answer, not an empty string.
    if(status_old = '', CAST(NULL AS Nullable(String)), status_old) AS prev_value_id,
    CAST(NULL AS Nullable(String)) AS prev_value_display,
    'insight_bitbucket_cloud' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM status_change
-- An update that changed neither status nor review state says nothing a
-- lifecycle consumer can use.
WHERE kind IN ('approval', 'changes_requested')
   OR status_new != ''
   OR COALESCE(update_state, '') != ''
