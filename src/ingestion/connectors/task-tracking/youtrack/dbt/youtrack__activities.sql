{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

WITH raw AS (
    SELECT coalesce(source_id, '') AS insight_source_id, coalesce(id, '') AS event_id,
        coalesce(_type, '') AS activity_type, author_id,
        fromUnixTimestamp64Milli(toInt64(assumeNotNull(timestamp))) AS event_at,
        {{ youtrack_json('target_json') }} AS target,
        {{ youtrack_json('field_json') }} AS field,
        {{ youtrack_json('added_json') }} AS added,
        {{ youtrack_json('removed_json') }} AS removed,
        {{ youtrack_json('activity_json') }} AS payload,
        toDateTime64(_airbyte_extracted_at, 3) AS collected_at
    FROM {{ source('bronze_youtrack', 'youtrack_activities') }} FINAL
    WHERE throwIf(timestamp IS NULL OR toInt64OrNull(timestamp) IS NULL OR id IS NULL OR source_id IS NULL,
                  'YouTrack activity requires identity and a millisecond timestamp') = 0
), field_types AS (
    SELECT insight_source_id, field_id, observed_at, field_type
    FROM {{ ref('youtrack__field_observations') }} FINAL
), first_field_types AS (
    SELECT insight_source_id, field_id, argMin(field_type, observed_at) AS field_type
    FROM field_types GROUP BY insight_source_id, field_id
), normalized AS (
    SELECT *, JSONExtractString(target, 'id') AS issue_id,
        multiIf(activity_type IN ('CustomFieldActivityItem', 'TextCustomFieldActivityItem'),
                    coalesce(nullIf(JSONExtractString(field, 'customField', 'id'), ''), JSONExtractString(field, 'id')),
                activity_type = 'TagsActivityItem', 'tags',
                activity_type = 'SprintActivityItem', 'sprints',
                activity_type = 'ProjectActivityItem', 'project',
                JSONExtractString(payload, 'targetMember') IN ('summary', 'description'), JSONExtractString(payload, 'targetMember'),
                '') AS field_id,
        if(activity_type = 'ProjectActivityItem', {{ youtrack_project_values('removed') }},
           {{ youtrack_values('removed') }}) AS before_pairs,
        if(activity_type = 'ProjectActivityItem', {{ youtrack_project_values('added') }},
           {{ youtrack_values('added') }}) AS after_pairs,
        arrayConcat({{ youtrack_value_types('removed') }}, {{ youtrack_value_types('added') }}) AS value_types
    FROM raw
    WHERE JSONExtractString(target, '$type') = 'Issue'
), typed AS (
    -- The issue properties carry the names the snapshot gives them, so one
    -- field keeps one name. `field.name` on a SprintActivityItem is the BOARD
    -- ("Board Automation Kanban"), not the property. A custom field's
    -- cardinality is its type's, which the activity item does not carry: the
    -- type observed last at or before the event, and before the first
    -- observation the first observed type. A later observation therefore never
    -- reinterprets an event it does not precede.
    SELECT n.insight_source_id AS insight_source_id, n.event_id AS event_id, n.activity_type AS activity_type,
        n.author_id AS author_id, n.event_at AS event_at, n.target AS target, n.field AS field,
        n.added AS added, n.removed AS removed, n.payload AS payload, n.collected_at AS collected_at,
        n.issue_id AS issue_id, n.field_id AS field_id,
        multiIf(n.field_id = 'sprints', 'Sprints', n.field_id = 'tags', 'Tags', n.field_id = 'project', 'Project',
                n.field_id = 'summary', 'Summary', n.field_id = 'description', 'Description',
                coalesce(nullIf(JSONExtractString(n.field, 'name'), ''), JSONExtractString(n.payload, 'targetMember'))) AS field_name,
        n.before_pairs AS before_pairs, n.after_pairs AS after_pairs, n.value_types AS value_types,
        multiIf(n.activity_type IN ('TagsActivityItem', 'SprintActivityItem'), 'multi',
                n.field_id IN ('project', 'summary', 'description'), 'single',
                coalesce(nullIf(t.field_type, ''), f.field_type, '') = '', 'unknown',
                endsWith(coalesce(nullIf(t.field_type, ''), f.field_type), '[*]'), 'multi',
                'single') AS field_cardinality,
        toUInt8(0) AS restated
    FROM normalized AS n
    ASOF LEFT JOIN field_types AS t
        ON t.insight_source_id = n.insight_source_id AND t.field_id = n.field_id AND n.event_at >= t.observed_at
    LEFT JOIN first_field_types AS f ON f.insight_source_id = n.insight_source_id AND f.field_id = n.field_id
    WHERE n.field_id != '' AND n.issue_id != '' AND n.event_id != ''
), sync_boards AS (
    -- A board whose sprints are managed through a field (isExplicit = false)
    -- records membership only as that field's change, never as a
    -- SprintActivityItem. The field's values are bundle values named after the
    -- board's sprints.
    SELECT coalesce(source_id, '') AS insight_source_id, assumeNotNull(id) AS agile_id,
        JSONExtractString({{ youtrack_json('agile_json') }}, 'sprintsSettings', 'sprintSyncField', 'id') AS sync_field_id,
        arrayMap(p -> JSONExtractString(p, 'id'), JSONExtractArrayRaw({{ youtrack_json('agile_json') }}, 'projects')) AS project_ids
    FROM {{ source('bronze_youtrack', 'youtrack_agiles') }} FINAL
    WHERE id IS NOT NULL
      AND NOT JSONExtractBool({{ youtrack_json('agile_json') }}, 'sprintsSettings', 'isExplicit')
      AND NOT JSONExtractBool({{ youtrack_json('agile_json') }}, 'sprintsSettings', 'disableSprints')
      AND sync_field_id != ''
), sync_sprints AS (
    -- (field, project, value name) -> the sprints of every such board that the
    -- project is on: one name spans many boards, and an issue sits in the
    -- sprint of each board its project belongs to.
    SELECT insight_source_id, field_id, project_id,
        mapFromArrays(groupArray(sprint_name), groupArray(sprints)) AS sprints_by_name
    FROM (
        SELECT b.insight_source_id AS insight_source_id, b.sync_field_id AS field_id, project_id,
            s.name AS sprint_name, arraySort(x -> x.1, groupUniqArray((s.id, s.name))) AS sprints
        FROM sync_boards AS b
        ARRAY JOIN b.project_ids AS project_id
        INNER JOIN (
            SELECT coalesce(source_id, '') AS insight_source_id, assumeNotNull(agile_id) AS agile_id,
                assumeNotNull(id) AS id, assumeNotNull(name) AS name
            FROM {{ source('bronze_youtrack', 'youtrack_sprints') }} FINAL
            WHERE id IS NOT NULL AND agile_id IS NOT NULL AND name IS NOT NULL
        ) AS s ON s.insight_source_id = b.insight_source_id AND s.agile_id = b.agile_id
        GROUP BY b.insight_source_id, b.sync_field_id, project_id, s.name
    )
    GROUP BY insight_source_id, field_id, project_id
), sync_membership AS (
    -- The same change restated as the `sprints` property, in sprint ids, so
    -- membership reads as one property whichever way a board manages it.
    SELECT a.insight_source_id AS insight_source_id, a.event_id AS event_id, a.activity_type AS activity_type,
        a.author_id AS author_id, a.event_at AS event_at, a.target AS target, a.field AS field,
        a.added AS added, a.removed AS removed, a.payload AS payload, a.collected_at AS collected_at,
        a.issue_id AS issue_id, 'sprints' AS field_id, 'Sprints' AS field_name,
        arraySort(x -> x.1, arrayDistinct(arrayFlatten(arrayMap(p -> l.sprints_by_name[p.2], a.before_pairs)))) AS before_pairs,
        arraySort(x -> x.1, arrayDistinct(arrayFlatten(arrayMap(p -> l.sprints_by_name[p.2], a.after_pairs)))) AS after_pairs,
        arrayMap(x -> 'opaque_id', arrayConcat(before_pairs, after_pairs)) AS value_types,
        'multi' AS field_cardinality,
        -- Shares its event_id with the field change it restates, which alone
        -- places the event in youtrack__activity_order.
        toUInt8(1) AS restated
    FROM typed AS a
    INNER JOIN {{ ref('youtrack__issues') }} AS i USING (insight_source_id, issue_id)
    INNER JOIN sync_sprints AS l
        ON l.insight_source_id = a.insight_source_id AND l.field_id = a.field_id AND l.project_id = i.project_id
    WHERE a.activity_type = 'CustomFieldActivityItem'
)
SELECT * FROM typed
UNION ALL
SELECT * FROM sync_membership
WHERE length(before_pairs) + length(after_pairs) > 0
