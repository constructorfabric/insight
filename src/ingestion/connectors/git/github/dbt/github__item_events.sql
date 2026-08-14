-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='incremental',
    unique_key='unique_key',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    schema='staging',
    tags=['github', 'silver:class_git_item_events']
) }}

-- GitHub names each change after the thing that changed rather than the field
-- it changed, so the event type is what carries the semantics; this maps the
-- vendor vocabulary onto the class's (field_id, delta_action, value) triple.
-- FINAL: the event is immutable but the names projected onto it are not — a
-- window re-fetch after a login or label rename re-emits the same key with
-- different values.
-- Only the two field-style events carry a previous value — everything else
-- reports the new state alone.
WITH pull_request_events AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        repo_full_name,
        'pull_request' AS item_type,
        item_number,
        event_id,
        event_type,
        event_at,
        actor_login,
        target_login,
        label_name,
        '' AS field_name,
        '' AS prev_value,
        '' AS new_value,
        state_reason,
        _airbyte_extracted_at
    FROM {{ source('bronze_github', 'pull_request_timeline_events') }} FINAL
    {% if is_incremental() %}
    WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
    {% endif %}
),
issue_events AS (
    SELECT
        tenant_id,
        source_id,
        unique_key,
        repo_full_name,
        'issue' AS item_type,
        item_number,
        event_id,
        event_type,
        event_at,
        actor_login,
        target_login,
        label_name,
        field_name,
        prev_value,
        new_value,
        state_reason,
        _airbyte_extracted_at
    FROM {{ source('bronze_github', 'issue_timeline_events') }} FINAL
    {% if is_incremental() %}
    WHERE _airbyte_extracted_at > (SELECT max(_airbyte_extracted_at) FROM {{ this }})
    {% endif %}
),
events AS (
    SELECT * FROM pull_request_events
    UNION ALL
    SELECT * FROM issue_events
)
SELECT
    tenant_id,
    source_id,
    unique_key,
    splitByChar('/', COALESCE(repo_full_name, ''))[1] AS project_key,
    splitByChar('/', COALESCE(repo_full_name, ''))[2] AS repo_slug,
    item_type,
    COALESCE(item_number, 0) AS item_number,
    COALESCE(event_id, '') AS event_id,
    parseDateTimeBestEffortOrNull(event_at) AS event_at,
    COALESCE(actor_login, '') AS actor_name,
    multiIf(
        event_type IN ('ClosedEvent', 'ReopenedEvent', 'MergedEvent'), 'state',
        event_type IN ('ReadyForReviewEvent', 'ConvertToDraftEvent'), 'draft',
        event_type IN ('AssignedEvent', 'UnassignedEvent'), 'assignee',
        event_type IN ('ReviewRequestedEvent', 'ReviewRequestRemovedEvent'), 'reviewer',
        event_type IN ('LabeledEvent', 'UnlabeledEvent'), 'label',
        event_type = 'ProjectV2ItemStatusChangedEvent', 'board_status',
        event_type = 'IssueTypeChangedEvent', 'issuetype',
        event_type = 'IssueFieldChangedEvent', lower(COALESCE(field_name, '')),
        ''
    ) AS field_id,
    multiIf(
        event_type IN ('AssignedEvent', 'ReviewRequestedEvent', 'LabeledEvent'), 'add',
        event_type IN ('UnassignedEvent', 'ReviewRequestRemovedEvent', 'UnlabeledEvent'), 'remove',
        'set'
    ) AS delta_action,
    multiIf(
        event_type = 'ClosedEvent', 'closed',
        event_type = 'ReopenedEvent', 'open',
        event_type = 'MergedEvent', 'merged',
        event_type = 'ReadyForReviewEvent', 'false',
        event_type = 'ConvertToDraftEvent', 'true',
        event_type IN ('AssignedEvent', 'UnassignedEvent', 'ReviewRequestedEvent', 'ReviewRequestRemovedEvent'), COALESCE(target_login, ''),
        event_type IN ('LabeledEvent', 'UnlabeledEvent'), COALESCE(label_name, ''),
        COALESCE(new_value, '')
    ) AS delta_value_id,
    -- A closed pull request or issue says why it closed; nothing else carries
    -- a display distinct from the value itself.
    multiIf(
        event_type = 'ClosedEvent', lower(COALESCE(state_reason, '')),
        ''
    ) AS delta_value_display,
    -- Only the board-status and field-change events report where they came
    -- from; NULL everywhere else is the honest answer, not an empty string.
    if(COALESCE(prev_value, '') = '', CAST(NULL AS Nullable(String)), prev_value) AS prev_value_id,
    CAST(NULL AS Nullable(String)) AS prev_value_display,
    'insight_github' AS data_source,
    toUnixTimestamp64Milli(now64()) AS _version,
    _airbyte_extracted_at
FROM events
