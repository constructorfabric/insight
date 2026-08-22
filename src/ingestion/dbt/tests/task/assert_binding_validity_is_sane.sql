-- A binding dated in the future silently does not apply, and two rows sharing
-- one (key, valid_from, recorded_at) make which decision won undefined. Both
-- are authoring mistakes that produce no error of their own.
--
-- `unique_key` carries the whole decision including `recorded_at`, so a genuine
-- correction is a distinct key; a collision here means two different decisions
-- were written as one.

SELECT 'future_valid_from' AS problem, insight_source_id, data_source, field_id, toString(valid_from) AS detail
FROM config.task_field_roles FINAL
WHERE is_deleted = 0 AND valid_from > now64(3) + INTERVAL 1 DAY

UNION ALL

SELECT 'future_valid_from', insight_source_id, data_source, field_id, toString(valid_from)
FROM config.task_value_map FINAL
WHERE is_deleted = 0 AND valid_from > now64(3) + INTERVAL 1 DAY

UNION ALL

SELECT 'colliding_decision', insight_source_id, data_source, field_id, unique_key
FROM (
    SELECT insight_source_id, data_source, field_id, unique_key, count() AS c
    FROM config.task_field_roles FINAL
    GROUP BY insight_source_id, data_source, field_id, unique_key
    HAVING c > 1
)

UNION ALL

SELECT 'colliding_decision', insight_source_id, data_source, field_id, unique_key
FROM (
    SELECT insight_source_id, data_source, field_id, unique_key, count() AS c
    FROM config.task_value_map FINAL
    GROUP BY insight_source_id, data_source, field_id, unique_key
    HAVING c > 1
)

LIMIT 100
