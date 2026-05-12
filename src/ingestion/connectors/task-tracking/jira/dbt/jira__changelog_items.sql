-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='table',
    alias='jira_changelog_items',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    settings={'allow_nullable_key': 1},
    tags=['staging', 'jira']
) }}

-- Materialized as `table` (not `incremental`) so every dbt run rewrites staging from scratch.
-- Rationale: the final GROUP BY collapses duplicates at the content-identity level, so
-- bronze append duplicates (same changelog re-emitted across Airbyte runs) produce one
-- staging row regardless.

-- Explode `bronze_jira.jira_issue_history.items` JSON array into one row per field change.
-- Consumed by `jira-enrich` Rust binary (reads from staging.jira_changelog_items).
--
-- Each history row has `changelog_id` and a JSON array `items` with elements shaped like:
--   { "field": "...", "fieldId": "...", "from": "...", "fromString": "...",
--     "to": "...", "toString": "..." }
--
-- ClickHouse strategy: arrayJoin() on JSONExtractArrayRaw, then JSONExtract* on each element.
-- Bronze dedup is handled by argMax on the natural key (source_id, changelog_id) picking the
-- latest Airbyte emission, then the final GROUP BY below collapses content-identical items
-- within a single changelog.

WITH exploded AS (
    SELECT
        COALESCE(h.source_id, '')                                AS insight_source_id,
        COALESCE(h.tenant_id, '')                                AS tenant_id,
        COALESCE(h.id_readable, '')                              AS id_readable,
        COALESCE(toString(h.changelog_id), '')                   AS changelog_id,
        COALESCE(parseDateTime64BestEffortOrNull(h.created_at, 3), toDateTime64(0, 3)) AS created_at,
        h.author_account_id                                      AS author_account_id,
        arrayJoin(JSONExtractArrayRaw(COALESCE(h.items, '[]')))  AS item_raw
    FROM (
        SELECT * FROM {{ source('bronze_jira', 'jira_issue_history') }}
        ORDER BY _airbyte_extracted_at DESC
        LIMIT 1 BY source_id, changelog_id
    ) h
    WHERE h.items IS NOT NULL AND h.items != '[]'
),
parsed AS (
    SELECT
        insight_source_id,
        tenant_id,
        id_readable,
        changelog_id,
        created_at,
        author_account_id,
        JSONExtractString(item_raw, 'fieldId')                 AS field_id,
        JSONExtractString(item_raw, 'field')                   AS field_name,
        nullIf(JSONExtractString(item_raw, 'from'), '')        AS value_from,
        nullIf(JSONExtractString(item_raw, 'fromString'), '')  AS value_from_string,
        nullIf(JSONExtractString(item_raw, 'to'), '')          AS value_to,
        nullIf(JSONExtractString(item_raw, 'toString'), '')    AS value_to_string
    FROM exploded
    -- Jira sometimes emits phantom changelog items with `fieldId=""` (typically system-level
    -- events like "WorklogId"/"RemoteIssueLink" that don't have a proper field mapping). The
    -- enrich binary drops them at runtime with a WARN; filter them here to keep the warning
    -- log quiet and save a wire round-trip.
    WHERE JSONExtractString(item_raw, 'fieldId') != ''
)
-- Dedup duplicates within a single changelog: Jira sometimes emits the same (fieldId, from/to)
-- twice in one items[] array. Group by the natural content-identity key.
-- `unique_key` encodes the same content-identity so silver/RMT dedup works on a single column.
SELECT
    CAST(concat(
        coalesce(insight_source_id, ''), '-',
        coalesce(changelog_id, ''), '-',
        coalesce(field_id, ''), '-',
        coalesce(value_from, ''), '-',
        coalesce(value_from_string, ''), '-',
        coalesce(value_to, ''), '-',
        coalesce(value_to_string, '')
    ) AS String) AS unique_key,
    insight_source_id,
    any(tenant_id)          AS tenant_id,
    id_readable,
    changelog_id,
    any(created_at)         AS created_at,
    any(author_account_id)  AS author_account_id,
    field_id,
    any(field_name)         AS field_name,
    value_from,
    value_from_string,
    value_to,
    value_to_string,
    toUnixTimestamp64Milli(now64(3))                           AS _version
FROM parsed
GROUP BY
    insight_source_id,
    id_readable,
    changelog_id,
    field_id,
    value_from,
    value_from_string,
    value_to,
    value_to_string
