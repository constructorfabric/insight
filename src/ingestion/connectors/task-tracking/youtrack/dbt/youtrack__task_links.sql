{{ config(materialized='table', schema='staging', tags=['youtrack', 'staging', 'silver:class_task_links'], engine='ReplacingMergeTree(_version)', order_by=['unique_key'], settings={'allow_nullable_key': 1}) }}

WITH observations AS (
    SELECT insight_source_id, issue_id, id_readable, observed_at,
        arrayFlatten(arrayMap(l -> arrayMap(t -> (
            concat(JSONExtractString(l, 'linkType', 'id'), ':', JSONExtractString(l, 'direction')),
            JSONExtractString(t, 'id'), JSONExtractString(t, 'idReadable')),
            JSONExtractArrayRaw(l, 'issues')), JSONExtractArrayRaw(payload, 'links'))) AS links
    FROM {{ ref('youtrack__issue_observations') }} FINAL
    -- trimmedIssues is a preview that YouTrack fills alongside the full `issues`;
    -- a set is trimmed only when the preview names more issues than the set holds.
    WHERE JSONHas(payload, 'links') AND NOT arrayExists(
        l -> length(JSONExtractArrayRaw(l, 'trimmedIssues')) > length(JSONExtractArrayRaw(l, 'issues')),
        JSONExtractArrayRaw(payload, 'links'))
), timelines AS (
    SELECT insight_source_id, issue_id,
        argMax(id_readable, observed_at) AS id_readable,
        arraySort(x -> x.1, groupArray((observed_at, links))) AS versions
    FROM observations GROUP BY insight_source_id, issue_id
), edges AS (
    SELECT *, arrayJoin(arrayDistinct(arrayMap(l -> (l.1, l.2), arrayFlatten(arrayMap(v -> v.2, versions))))) AS link
    FROM timelines
), spans AS (
    SELECT *, arrayJoin(arrayFilter(n -> arrayExists(l -> l.1 = link.1 AND l.2 = link.2, versions[n].2)
        AND (n = 1 OR NOT arrayExists(l -> l.1 = link.1 AND l.2 = link.2, versions[n-1].2)), range(1, length(versions)+1))) AS start_index
    FROM edges
), bounds AS (
    SELECT *, arrayFirstIndex(v -> NOT arrayExists(l -> l.1 = link.1 AND l.2 = link.2, v.2), arraySlice(versions, start_index)) AS end_offset
    FROM spans
)
SELECT assumeNotNull(concat(insight_source_id, '-youtrack-', issue_id, '-', link.1, '-', link.2, '-', toString(versions[start_index].1))) AS unique_key,
    insight_source_id, 'youtrack' AS data_source, id_readable,
    link.1 AS link_type, CAST('issue' AS Enum8('issue' = 1, 'pull_request' = 2)) AS target_type,
    arrayLast(l -> l.1 = link.1 AND l.2 = link.2, arrayFlatten(arrayMap(v -> v.2, versions))).3 AS target_readable,
    CAST(splitByChar('-', id_readable)[1] != splitByChar('-', target_readable)[1] AS Bool) AS is_cross_repository,
    versions[start_index].1 AS valid_from, toUInt8(0) AS valid_from_known,
    if(end_offset = 0, CAST(NULL AS Nullable(DateTime64(3))), toNullable(versions[start_index+end_offset-1].1)) AS valid_to,
    CAST(NULL AS Nullable(String)) AS added_by, CAST(NULL AS Nullable(String)) AS removed_by,
    CAST('observation' AS Enum8('event' = 1, 'observation' = 2)) AS evidence,
    CAST(NULL AS Nullable(String)) AS origin_event_id,
    versions[-1].1 AS collected_at, toUInt64(toUnixTimestamp64Milli(now64(3))) AS _version,
    CAST(issue_id AS Nullable(String)) AS issue_id,
    CAST(link.2 AS Nullable(String)) AS target_id
FROM bounds
