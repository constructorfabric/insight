{{ config(
    materialized='table',
    full_refresh=false,
    alias='jira__issue_link_snapshot',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    query_settings={'join_use_nulls': 1},
    tags=['staging', 'jira']
) }}

-- The links and the parent each issue holds now, one row per link object, with
-- the linked issue named by its immutable id and its key. Read by
-- `jira__task_links`, which seeds the links no changelog event reaches, and by
-- `jira__issue_key_ids`, for which every row is a key the linked issue has.
-- Earlier rows omitted while the other issue was hidden retain their original extraction time.

{% set previous = load_relation(this) if execute else none %}

WITH held AS (
    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        kind,
        link_id,
        type_id,
        type_name,
        type_outward,
        type_inward,
        direction,
        relation,
        target_id,
        target_key
    FROM {{ ref('jira__issue_link_current') }} FINAL
),

issue_reads AS (
    SELECT
        insight_source_id,
        issue_id,
        read_at
    FROM {{ ref('jira__issue_read_current') }} FINAL
),

held_keys AS (
    SELECT
        insight_source_id AS held_source_id,
        issue_id AS held_issue_id,
        kind AS held_kind,
        link_id AS held_link_id
    FROM held
),

missing_rows AS (
    {% if previous is not none %}
    SELECT
        p.insight_source_id AS insight_source_id,
        p.issue_id AS issue_id,
        p.id_readable AS id_readable,
        p.extracted_at AS extracted_at,
        p.kind AS kind,
        p.link_id AS link_id,
        p.type_id AS type_id,
        p.type_name AS type_name,
        p.type_outward AS type_outward,
        p.type_inward AS type_inward,
        p.direction AS direction,
        p.relation AS relation,
        p.target_id AS target_id,
        p.target_key AS target_key
    FROM {{ previous }} AS p FINAL
    LEFT ANTI JOIN held_keys AS h
        ON h.held_source_id = p.insight_source_id
       AND h.held_issue_id = p.issue_id
       AND h.held_kind = p.kind
       AND h.held_link_id = p.link_id
    {% else %}
    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        kind,
        link_id,
        type_id,
        type_name,
        type_outward,
        type_inward,
        direction,
        relation,
        target_id,
        target_key
    FROM held
    WHERE 0
    {% endif %}
),

missing_links AS (
    SELECT
        m.insight_source_id AS insight_source_id,
        m.issue_id AS issue_id,
        m.id_readable AS id_readable,
        m.extracted_at AS extracted_at,
        m.kind AS kind,
        m.link_id AS link_id,
        m.type_id AS type_id,
        m.type_name AS type_name,
        m.type_outward AS type_outward,
        m.type_inward AS type_inward,
        m.direction AS direction,
        m.relation AS relation,
        m.target_id AS target_id,
        m.target_key AS target_key,
        r.read_at AS read_at
    FROM missing_rows AS m
    LEFT JOIN issue_reads AS r
        ON r.insight_source_id = m.insight_source_id
       AND r.issue_id = m.issue_id
),

visible_links AS (
    {{ jira_link_visibility('missing_links', 'missing_rows') }}
),

remembered AS (
    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        kind,
        link_id,
        type_id,
        type_name,
        type_outward,
        type_inward,
        direction,
        relation,
        target_id,
        target_key
    FROM held

    UNION ALL

    SELECT
        insight_source_id,
        issue_id,
        id_readable,
        extracted_at,
        kind,
        link_id,
        type_id,
        type_name,
        type_outward,
        type_inward,
        direction,
        relation,
        target_id,
        target_key
    FROM visible_links
    WHERE read_at IS NOT NULL AND NOT other_issue_visible
)

SELECT
    CAST(concat(insight_source_id, '-', issue_id, '-', kind, '-', link_id, '-', target_id, '-', target_key) AS String) AS unique_key,
    insight_source_id,
    issue_id,
    id_readable,
    kind,
    link_id,
    type_id,
    type_name,
    type_outward,
    type_inward,
    direction,
    relation,
    target_id,
    target_key,
    extracted_at,
    toUnixTimestamp64Milli(now64(3))                                  AS _version
FROM remembered
