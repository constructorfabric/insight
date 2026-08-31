-- depends_on: {{ ref('github__bronze_promoted') }}
{{ config(
    materialized='table',
    alias='github__task_field_history',
    schema='staging',
    tags=['github', 'staging', 'silver:class_task_field_history']
) }}

-- Per-(issue x field x event) history; unioned into
-- `silver.class_task_field_history` via `union_by_tag`. What `jira-enrich`
-- computes in Rust for Jira is expressible here in SQL, because GitHub's
-- change events carry the whole previous value rather than a delta.
--
-- A timeline records changes, so a field set at creation and never touched
-- never appears in it. One rule produces every initial value: the earliest
-- event's previous value if the field has events, the snapshot value if it has
-- none. Nothing is folded and nothing is replayed.
--
-- Materialized as a table, not incrementally: an initial row depends on the
-- whole event set of its issue, so an incremental build has to reprocess whole
-- issues rather than new events. Worth doing, but not before the model is
-- proven — and when it is done, key `delete+insert` on the issue.
--
-- INVARIANT: `field_id` is the vendor's own identifier, never a role. Gold
-- resolves a role through `config.task_field_roles`; putting a role here would
-- bake one deployment's interpretation into the record of what happened.
--
-- One field identifier is composed rather than quoted: a board's status is
-- `project_status:<project node id>`. GitHub names no field on a board status
-- event — it states the board and the two column names — and every board
-- defines its own status field, so the board is the field's identity. It is
-- NOT the issue's `state`: an issue's open/closed lifecycle and a board column
-- are different fields, bound separately, and nothing here merges them.

WITH
issues AS (
    SELECT
        tenant_id,
        source_id,
        toString(id)                                            AS issue_id,
        concat(repo_full_name, '#', toString(number))           AS id_readable,
        title,
        repo_full_name,
        number,
        parseDateTimeBestEffortOrNull(created_at)               AS created_at,
        author_login,
        toString(author_id)                                     AS author_id,
        state,
        -- GraphQL states the reason as an upper-case enum while REST states it
        -- lower-case. Same lifecycle value, two spellings — normalised here so
        -- an operator binds three values, not six.
        lower(COALESCE(state_reason, ''))                       AS state_reason,
        COALESCE(issue_type_id, '')                             AS issue_type_id,
        COALESCE(issue_field_values_json, '[]')                 AS field_values_json,
        splitByChar(',', COALESCE(assignee_ids, ''))            AS assignee_ids,
        _airbyte_extracted_at
    FROM {{ source('bronze_github', 'issues') }} FINAL
    -- The alias is already the parsed value; an unparseable timestamp is NULL,
    -- and an issue with no creation time cannot anchor its own history.
    WHERE created_at IS NOT NULL
),

-- The catalogue bridges the two identifier spaces: a snapshot value names its
-- field by the numeric id, a change event by the node id.
field_catalogue AS (
    SELECT
        tenant_id,
        source_id,
        field_id,
        field_database_id,
        field_name
    FROM {{ source('bronze_github', 'issue_fields') }} FINAL
),

-- Every timeline event mapped onto (field_id, value). `field_id` is the
-- vendor's name for the thing that changed: GitHub names an event after the
-- change, not after the field, so the event type is what carries it.
events AS (
    SELECT
        e.tenant_id                                             AS tenant_id,
        e.source_id                                             AS source_id,
        e.repo_full_name                                        AS repo_full_name,
        e.item_number                                           AS number,
        e.event_id                                              AS event_id,
        parseDateTimeBestEffortOrNull(e.event_at)               AS event_at,
        e.actor_login                                           AS actor_login,
        toString(e.actor_id)                                    AS actor_id,
        multiIf(
            e.event_type IN ('ClosedEvent', 'ReopenedEvent'),          'state',
            e.event_type IN ('AssignedEvent', 'UnassignedEvent'),      'assignees',
            e.event_type = 'IssueTypeChangedEvent',                    'type',
            e.event_type = 'IssueFieldChangedEvent',                   COALESCE(e.field_id, ''),
            -- A board's status is its OWN field, never the issue's `state`.
            -- Every board defines its own, so the board IS the field identity:
            -- the event says "on board X, status went A to B" and names no
            -- field object. Deriving which ProjectV2 field object it was would
            -- mean guessing; the board is stated and sufficient.
            e.event_type = 'ProjectV2ItemStatusChangedEvent',
                if(COALESCE(e.project_id, '') = '', '', concat('project_status:', e.project_id)),
            ''
        )                                                       AS field_id,
        multiIf(
            e.event_type = 'ClosedEvent',   concat('closed:', lower(COALESCE(e.state_reason, ''))),
            e.event_type = 'ReopenedEvent', 'open',
            e.event_type = 'AssignedEvent', toString(e.target_id),
            -- Removing an assignee states no remaining owner. An empty value
            -- reaches gold as an unresolvable account and correctly leaves the
            -- issue unattributed; see the design's assignee limitation.
            e.event_type = 'UnassignedEvent', '',
            e.event_type = 'IssueTypeChangedEvent', COALESCE(e.new_value_id, ''),
            e.event_type = 'IssueFieldChangedEvent', COALESCE(e.new_value, ''),
            -- The value key carries the board too. `class_task_statuses.status_id`
            -- is unique per source and gold joins it to this column, so a bare
            -- column name would collapse two boards' same-named columns into
            -- one dimension row — and board option identifiers are inherited
            -- from a template, so they collide as well. Lower-cased because a
            -- board states its own casing and two spellings are one column.
            e.event_type = 'ProjectV2ItemStatusChangedEvent',
                if(COALESCE(e.new_value, '') = '', '',
                   concat(COALESCE(e.project_id, ''), ':', lower(e.new_value))),
            ''
        )                                                       AS value_id,
        multiIf(
            e.event_type = 'IssueTypeChangedEvent', COALESCE(e.new_value, ''),
            e.event_type = 'AssignedEvent',         COALESCE(e.target_login, ''),
            COALESCE(e.new_value, '')
        )                                                       AS value_display,
        multiIf(
            e.event_type = 'ClosedEvent',           'open',
            e.event_type = 'ReopenedEvent',         concat('closed:', lower(COALESCE(e.state_reason, ''))),
            e.event_type = 'IssueTypeChangedEvent', COALESCE(e.prev_value_id, ''),
            -- The first status event of a card states an empty previous value,
            -- which must stay empty: `initial_values` reads it as the value at
            -- creation and a composed key over nothing is not a value.
            e.event_type = 'ProjectV2ItemStatusChangedEvent',
                if(COALESCE(e.prev_value, '') = '', '',
                   concat(COALESCE(e.project_id, ''), ':', lower(e.prev_value))),
            COALESCE(e.prev_value, '')
        )                                                       AS prev_value_id,
        e._airbyte_extracted_at                                 AS _airbyte_extracted_at
    FROM {{ source('bronze_github', 'issue_timeline_events') }} AS e FINAL
    WHERE e.event_type IN (
        'ClosedEvent', 'ReopenedEvent', 'AssignedEvent', 'UnassignedEvent',
        'IssueTypeChangedEvent', 'IssueFieldChangedEvent',
        'ProjectV2ItemStatusChangedEvent'
    )
),

scoped_events AS (
    SELECT
        i.tenant_id                                             AS tenant_id,
        i.source_id                                             AS source_id,
        i.issue_id                                              AS issue_id,
        i.id_readable                                           AS id_readable,
        ev.event_id                                             AS event_id,
        ev.event_at                                             AS event_at,
        ev.actor_login                                          AS actor_login,
        ev.actor_id                                             AS actor_id,
        ev.field_id                                             AS field_id,
        ev.value_id                                             AS value_id,
        ev.value_display                                        AS value_display,
        ev.prev_value_id                                        AS prev_value_id,
        ev._airbyte_extracted_at                                AS _airbyte_extracted_at
    FROM events AS ev
    INNER JOIN issues AS i
        ON i.tenant_id = ev.tenant_id
        AND i.source_id = ev.source_id
        AND i.repo_full_name = ev.repo_full_name
        AND i.number = ev.number
    WHERE ev.field_id != '' AND ev.event_at IS NOT NULL
),

-- Snapshot values, one row per (issue, field). These are the fallback when a
-- field never changed; where it did change, the earliest event's previous
-- value wins because it states the value at creation.
snapshot_values AS (
    SELECT
        i.tenant_id,
        i.source_id,
        i.issue_id,
        i.id_readable,
        i.created_at,
        i.author_login,
        i.author_id,
        'state'                                                 AS field_id,
        'open'                                                  AS value_id,
        'open'                                                  AS value_display,
        i._airbyte_extracted_at
    FROM issues AS i

    UNION ALL

    SELECT
        i.tenant_id, i.source_id, i.issue_id, i.id_readable, i.created_at,
        i.author_login, i.author_id,
        'assignees'                                             AS field_id,
        trimBoth(i.assignee_ids[1])                             AS value_id,
        ''                                                      AS value_display,
        i._airbyte_extracted_at
    FROM issues AS i
    WHERE trimBoth(i.assignee_ids[1]) != ''

    UNION ALL

    SELECT
        i.tenant_id, i.source_id, i.issue_id, i.id_readable, i.created_at,
        i.author_login, i.author_id,
        'type'                                                  AS field_id,
        i.issue_type_id                                         AS value_id,
        ''                                                      AS value_display,
        i._airbyte_extracted_at
    FROM issues AS i
    WHERE i.issue_type_id != ''

    UNION ALL

    -- Native issue-field values, resolved from the numeric identifier the REST
    -- payload states to the node id the timeline states.
    SELECT
        i.tenant_id, i.source_id, i.issue_id, i.id_readable, i.created_at,
        i.author_login, i.author_id,
        c.field_id                                              AS field_id,
        JSONExtractString(v, 'value')                           AS value_id,
        ''                                                      AS value_display,
        i._airbyte_extracted_at
    FROM issues AS i
    ARRAY JOIN JSONExtractArrayRaw(i.field_values_json)         AS v
    INNER JOIN field_catalogue AS c
        ON c.tenant_id = i.tenant_id
        AND c.source_id = i.source_id
        AND c.field_database_id = JSONExtractString(v, 'issue_field_id')
    WHERE JSONExtractString(v, 'value') != ''
),

-- The one rule: earliest event's previous value, else the snapshot value.
initial_values AS (
    SELECT
        s.tenant_id,
        s.source_id,
        s.issue_id,
        s.id_readable,
        s.created_at,
        s.author_login,
        s.author_id,
        s.field_id,
        COALESCE(e.first_prev, s.value_id)                      AS value_id,
        s.value_display,
        s._airbyte_extracted_at
    FROM snapshot_values AS s
    LEFT JOIN (
        SELECT
            tenant_id, source_id, issue_id, field_id,
            argMin(prev_value_id, event_at)                     AS first_prev
        FROM scoped_events
        GROUP BY tenant_id, source_id, issue_id, field_id
    ) AS e
        ON e.tenant_id = s.tenant_id
        AND e.source_id = s.source_id
        AND e.issue_id = s.issue_id
        AND e.field_id = s.field_id

    UNION ALL

    -- A field the snapshot no longer carries but history changed still has a
    -- value at creation, and gold needs it to read the issue's whole life.
    SELECT
        e.tenant_id, e.source_id, e.issue_id, e.id_readable,
        i.created_at, i.author_login, i.author_id,
        e.field_id,
        e.first_prev                                            AS value_id,
        ''                                                      AS value_display,
        e._airbyte_extracted_at
    FROM (
        SELECT
            tenant_id, source_id, issue_id, id_readable, field_id,
            argMin(prev_value_id, event_at)                     AS first_prev,
            max(_airbyte_extracted_at)                          AS _airbyte_extracted_at
        FROM scoped_events
        GROUP BY tenant_id, source_id, issue_id, id_readable, field_id
    ) AS e
    INNER JOIN issues AS i
        ON i.tenant_id = e.tenant_id AND i.source_id = e.source_id AND i.issue_id = e.issue_id
    LEFT ANTI JOIN snapshot_values AS s
        ON s.tenant_id = e.tenant_id
        AND s.source_id = e.source_id
        AND s.issue_id = e.issue_id
        AND s.field_id = e.field_id
    -- Board status is excluded from initial synthesis on purpose. An initial
    -- row is dated at the issue's creation, but a card can join a board long
    -- after — synthesising one would fabricate a span the issue never had.
    -- Nothing is lost: adding a card emits its own status event, whose
    -- previous value is empty.
    WHERE e.first_prev != '' AND NOT startsWith(e.field_id, 'project_status:')
),

-- Row 1 of every issue: who created it and when. A sentinel, not a field —
-- gold reads `created_at` as the earliest synthetic_initial event and nothing
-- else looks at it.
creation_marker AS (
    SELECT
        i.tenant_id,
        i.source_id,
        i.issue_id,
        i.id_readable,
        concat('initial:', i.issue_id)                          AS event_id,
        toDateTime64(i.created_at, 3)                           AS event_at,
        'synthetic_initial'                                     AS event_kind,
        toUInt32(0)                                             AS _seq,
        i.author_id                                             AS author_id,
        i.author_login                                          AS author_display,
        'created'                                               AS field_id,
        ''                                                      AS value_id,
        ''                                                      AS value_display,
        i._airbyte_extracted_at
    FROM issues AS i
),

initial_rows AS (
    SELECT
        v.tenant_id,
        v.source_id,
        v.issue_id,
        v.id_readable,
        concat('initial:', v.issue_id)                          AS event_id,
        toDateTime64(v.created_at, 3)                           AS event_at,
        'synthetic_initial'                                     AS event_kind,
        toUInt32(row_number() OVER (
            PARTITION BY v.tenant_id, v.source_id, v.issue_id ORDER BY v.field_id
        ))                                                      AS _seq,
        v.author_id,
        v.author_login                                          AS author_display,
        v.field_id,
        v.value_id,
        v.value_display,
        v._airbyte_extracted_at
    FROM initial_values AS v
    WHERE v.value_id != ''
),

changelog_rows AS (
    SELECT
        tenant_id,
        source_id,
        issue_id,
        id_readable,
        event_id,
        toDateTime64(event_at, 3)                               AS event_at,
        'changelog'                                             AS event_kind,
        toUInt32(0)                                             AS _seq,
        actor_id                                                AS author_id,
        actor_login                                             AS author_display,
        field_id,
        value_id,
        value_display,
        _airbyte_extracted_at
    FROM scoped_events
),

every_row AS (
    SELECT * FROM creation_marker
    UNION ALL
    SELECT * FROM initial_rows
    UNION ALL
    SELECT * FROM changelog_rows
)

SELECT
    CAST(concat(source_id, '-github-', id_readable, '-', field_id, '-', event_id) AS String) AS unique_key,
    CAST(source_id AS String)                                   AS insight_source_id,
    CAST('github' AS String)                                    AS data_source,
    CAST(issue_id AS String)                                    AS issue_id,
    CAST(id_readable AS String)                                 AS id_readable,
    -- One row per (issue x field x event) all share the issue's summary; it is
    -- an issue attribute, not an event one, so it is joined in rather than
    -- repeated by every branch of the union above.
    CAST(nullIf(issue_title.title, '') AS Nullable(String))     AS title,
    CAST(event_id AS String)                                    AS event_id,
    -- The WHERE below already excludes the unparseable ones; the class declares
    -- a non-Nullable column and `union_by_tag` fails the shared relation for
    -- every source if one branch widens it.
    assumeNotNull(event_at)                                     AS event_at,
    -- Superset enum: the jira arms of this union also emit 'availability' and
    -- 'lifecycle' rows, and UNION ALL needs one common enum type across arms.
    CAST(event_kind AS Enum8('changelog' = 1, 'synthetic_initial' = 2, 'availability' = 3, 'lifecycle' = 4)) AS event_kind,
    _seq                                                        AS _seq,
    CAST(nullIf(author_id, '0') AS Nullable(String))            AS author_id,
    CAST(nullIf(author_display, '') AS Nullable(String))        AS author_display,
    CAST(field_id AS String)                                    AS field_id,
    CAST(field_id AS String)                                    AS field_name,
    CAST('single' AS Enum8('single' = 1, 'multi' = 2))          AS field_cardinality,
    CAST('set' AS Enum8('set' = 1, 'add' = 2, 'remove' = 3))    AS delta_action,
    CAST(nullIf(value_id, '') AS Nullable(String))              AS delta_value_id,
    CAST(nullIf(value_display, '') AS Nullable(String))         AS delta_value_display,
    CAST([value_id] AS Array(String))                           AS value_ids,
    CAST([if(value_display != '', value_display, value_id)] AS Array(String)) AS value_displays,
    CAST(
        multiIf(
            field_id = 'assignees', 'account_id',
            field_id = 'state', 'string_literal',
            -- A board column is named, not identified: history states the
            -- display name and nothing else.
            startsWith(field_id, 'project_status:'), 'string_literal',
            'opaque_id'
        )
        AS Enum8('opaque_id' = 1, 'account_id' = 2, 'string_literal' = 3, 'path' = 4, 'none' = 5)
    )                                                           AS value_id_type,
    toDateTime64(_airbyte_extracted_at, 3)                      AS collected_at,
    CAST(toUnixTimestamp64Milli(toDateTime64(_airbyte_extracted_at, 3)) AS UInt64) AS _version
FROM every_row
LEFT JOIN (SELECT issue_id, title FROM issues) AS issue_title
    ON issue_title.issue_id = every_row.issue_id
WHERE event_at IS NOT NULL
