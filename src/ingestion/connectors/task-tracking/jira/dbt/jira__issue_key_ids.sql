{{ config(
    materialized='table',
    alias='jira__issue_key_ids',
    schema='staging',
    engine=insight_engine('ReplacingMergeTree', '_version'),
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['staging', 'jira']
) }}

-- Every key an issue is known to have had, with the issue's immutable id. The
-- changelog names a linked issue only by the key it had when the link changed,
-- so `jira__task_links` and the journal's `issuelinks` resolve that key here.
--
-- INVARIANT: one id per key. Jira never reassigns a key: a moved issue keeps
-- its old key as an alias. Where two inputs disagree anyway, the issue's own
-- synced rows win, then the census, then what another issue's data says.
--
-- A key no input names has no row: an issue in a project the connector cannot
-- read and that no synced issue's snapshot links to, one deleted before it was
-- ever read, or an earlier key of an issue whose own changelog is not synced.

-- INVARIANT: the destination adds `id_readable` to the census on the first sync
-- after reconcile publishes the manifest that declares it; a sync that runs
-- before that must still build, so the census arm waits for the column.
{%- set census_columns = adapter.get_columns_in_relation(source('bronze_jira', 'jira_issue_census')) if execute else [] %}
{%- set census_has_keys = 'id_readable' in (census_columns | map(attribute='name') | list) %}

WITH issue_rows AS (
    -- Every row, not the newest: bronze keeps one per key the connector saw.
    SELECT
        tenant_id,
        CAST(source_id AS String)                                     AS insight_source_id,
        COALESCE(toString(id_readable), '')                           AS issue_key,
        CAST(jira_id AS String)                                       AS issue_id,
        toUInt8(1)                                                    AS precedence
    FROM {{ source('bronze_jira', 'jira_issue') }}

    UNION ALL

    SELECT
        tenant_id,
        CAST(source_id AS String)                                     AS insight_source_id,
        COALESCE(toString(id_readable), '')                           AS issue_key,
        CAST(jira_id AS String)                                       AS issue_id,
        toUInt8(1)                                                    AS precedence
    FROM {{ source('bronze_jira', 'jira_issue_keys') }}
{%- if census_has_keys %}

    UNION ALL

    -- The census lists every issue a visible project holds, whatever its last
    -- update, so a key outside the sync window still resolves.
    SELECT
        tenant_id,
        CAST(source_id AS String)                                     AS insight_source_id,
        COALESCE(toString(id_readable), '')                           AS issue_key,
        CAST(jira_id AS String)                                       AS issue_id,
        toUInt8(2)                                                    AS precedence
    FROM {{ source('bronze_jira', 'jira_issue_census') }}
{%- endif %}
),

-- A move is a `Key` item on the moved issue's own changelog, with no field id,
-- which `jira__changelog_items` leaves out: both of its sides are keys of the
-- issue the entry belongs to.
moves AS (
    SELECT
        tenant_id,
        insight_source_id,
        arrayJoin([
            JSONExtractString(item, 'fromString'),
            JSONExtractString(item, 'from'),
            JSONExtractString(item, 'toString'),
            JSONExtractString(item, 'to')
        ])                                                            AS issue_key,
        issue_id,
        toUInt8(3)                                                    AS precedence
    FROM (
        SELECT
            tenant_id,
            CAST(source_id AS String)                                 AS insight_source_id,
            COALESCE(toString(jira_id), '')                           AS issue_id,
            arrayJoin(JSONExtractArrayRaw(COALESCE(items, '[]')))     AS item
        FROM {{ source('bronze_jira', 'jira_issue_history') }}
        -- A substring probe before any JSON is parsed: most entries move nothing.
        WHERE position(items, 'Key') > 0
    )
    WHERE JSONExtractString(item, 'field') = 'Key'
      AND JSONExtractString(item, 'fieldtype') = 'jira'
      AND JSONExtractString(item, 'fieldId') = ''
),

linked_now AS (
    SELECT
        tenant_id,
        insight_source_id,
        target_key                                                    AS issue_key,
        target_id                                                     AS issue_id,
        toUInt8(4)                                                    AS precedence
    FROM {{ ref('jira__issue_link_snapshot') }} FINAL
),

-- A parent change names the parent by id and by the key it had then.
parent_changes AS (
    SELECT
        tenant_id,
        insight_source_id,
        arrayJoin([
            (COALESCE(value_from_string, ''), COALESCE(value_from, '')),
            (COALESCE(value_to_string, ''), COALESCE(value_to, ''))
        ])                                                            AS pair,
        pair.1                                                        AS issue_key,
        pair.2                                                        AS issue_id,
        toUInt8(4)                                                    AS precedence
    FROM {{ ref('jira__changelog_items') }}
    WHERE field_id = 'parent'
),

every_pair AS (
    SELECT tenant_id, insight_source_id, issue_key, issue_id, precedence FROM issue_rows
    UNION ALL
    SELECT tenant_id, insight_source_id, issue_key, issue_id, precedence FROM moves
    UNION ALL
    SELECT tenant_id, insight_source_id, issue_key, issue_id, precedence FROM linked_now
    UNION ALL
    SELECT tenant_id, insight_source_id, issue_key, issue_id, precedence FROM parent_changes
),

-- A key carries its project prefix and an id is all digits; a pair that is not
-- one of each names no issue.
well_formed AS (
    SELECT tenant_id, insight_source_id, issue_key, issue_id, precedence
    FROM every_pair
    WHERE match(issue_key, '^[^-]+-[0-9]+$')
      AND match(issue_id, '^[0-9]+$')
)

SELECT
    CAST(concat(ifNull(tenant_id, ''), '-', insight_source_id, '-', issue_key) AS String) AS unique_key,
    tenant_id,
    insight_source_id,
    issue_key,
    argMin(issue_id, (precedence, issue_id))                          AS issue_id,
    toUnixTimestamp64Milli(now64(3))                                  AS _version
FROM well_formed
GROUP BY tenant_id, insight_source_id, issue_key
