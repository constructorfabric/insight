-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='view',
    alias='github__issue_link_events',
    schema='staging',
    tags=['github', 'staging']
) }}

-- Every link between work items that appeared or disappeared, as a delta.
--
-- A link event states only what changed, never the set that resulted, so the
-- links an issue held at a given moment are a fold over these rows. That fold
-- is `github__task_links`; this model only normalises the vendor's twelve
-- event types into one shape.
--
-- Both ends of a link emit their own event at the same instant — a parent
-- records SubIssue*, its child records ParentIssue* — so a link appears twice
-- in this table, once from each side, and each side is true on its own terms.
-- Deduplicating to a single canonical direction would make the parent's
-- history depend on whether the child was collected.

WITH linked AS (
    SELECT
        COALESCE(source_id, '')                                 AS insight_source_id,
        COALESCE(tenant_id, '')                                 AS tenant_id,
        COALESCE(event_id, '')                                  AS event_id,
        COALESCE(event_type, '')                                AS event_type,
        parseDateTimeBestEffortOrNull(event_at)                 AS event_at,
        concat(COALESCE(repo_full_name, ''), '#', toString(COALESCE(item_number, 0))) AS id_readable,
        COALESCE(repo_full_name, '')                            AS source_repo,
        COALESCE(actor_login, '')                               AS actor_login,
        toString(COALESCE(actor_id, 0))                         AS actor_id,
        COALESCE(link_target_type, '')                          AS target_type_raw,
        COALESCE(link_target_repo_full_name, '')                AS target_repo,
        COALESCE(link_target_number, 0)                         AS target_number,
        COALESCE(is_cross_repository, false)                    AS is_cross_repository,
        _airbyte_extracted_at
    FROM {{ source('bronze_github', 'issue_timeline_events') }} FINAL
    WHERE event_type IN (
        'SubIssueAddedEvent', 'SubIssueRemovedEvent',
        'ParentIssueAddedEvent', 'ParentIssueRemovedEvent',
        'BlockedByAddedEvent', 'BlockedByRemovedEvent',
        'BlockingAddedEvent', 'BlockingRemovedEvent',
        'ConnectedEvent', 'DisconnectedEvent',
        'MarkedAsDuplicateEvent', 'UnmarkedAsDuplicateEvent'
    )
)

SELECT
    CAST(concat(insight_source_id, '-github-link-', event_id) AS String)    AS unique_key,
    CAST(insight_source_id AS String)                                      AS insight_source_id,
    CAST(tenant_id AS String)                                              AS tenant_id,
    CAST('github' AS String)                                               AS data_source,
    CAST(id_readable AS String)                                            AS id_readable,
    CAST(event_id AS String)                                               AS event_id,
    assumeNotNull(event_at)                                                AS event_at,
    CAST(multiIf(
        event_type IN ('SubIssueAddedEvent', 'SubIssueRemovedEvent'),           'sub_issue',
        event_type IN ('ParentIssueAddedEvent', 'ParentIssueRemovedEvent'),     'parent',
        event_type IN ('BlockedByAddedEvent', 'BlockedByRemovedEvent'),         'blocked_by',
        event_type IN ('BlockingAddedEvent', 'BlockingRemovedEvent'),           'blocking',
        event_type IN ('MarkedAsDuplicateEvent', 'UnmarkedAsDuplicateEvent'),   'duplicate_of',
        'connected'
    ) AS String)                                                           AS link_type,
    -- The vendor spells the direction in the type name and nowhere else.
    CAST(if(
        event_type IN ('SubIssueRemovedEvent', 'ParentIssueRemovedEvent',
                       'BlockedByRemovedEvent', 'BlockingRemovedEvent',
                       'DisconnectedEvent', 'UnmarkedAsDuplicateEvent'),
        'remove', 'add') AS String)                                        AS link_action,
    CAST(if(lower(target_type_raw) = 'pullrequest', 'pull_request', 'issue') AS String) AS target_type,
    CAST(concat(target_repo, '#', toString(target_number)) AS String)      AS target_readable,
    -- Derived, not read: only the connect and duplicate pairs report
    -- `isCrossRepository`, so trusting the vendor flag would call every
    -- cross-repository parent, sub-issue or dependency a same-repository one.
    -- The two repositories are on the row either way.
    CAST(source_repo != target_repo AS Bool)                               AS is_cross_repository,
    CAST(nullIf(actor_login, '') AS Nullable(String))                      AS actor_display,
    CAST(nullIf(actor_id, '0') AS Nullable(String))                        AS actor_id,
    toDateTime64(_airbyte_extracted_at, 3)                                 AS collected_at,
    CAST(toUnixTimestamp64Milli(toDateTime64(_airbyte_extracted_at, 3)) AS UInt64) AS _version
FROM linked
-- A link event with no target is a payload the query did not ask for; keeping
-- it would put a row with no other end into the fold.
WHERE event_at IS NOT NULL AND target_number != 0
