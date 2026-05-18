-- After ReplacingMergeTree merge there must be at most one row per
-- (tenant, person_key, date) in `silver.class_collab_meeting_activity`,
-- regardless of how many `data_source` / `insight_source_id` values exist.
--
-- Multiple rows for the same person on the same date almost always mean
-- a parallel/duplicate stream slipped through — see issue #283 for the
-- canonical case (a tenant with two Airbyte Zoom sources, `main` and
-- `zoom-main`, both ingesting the same Zoom tenant). They can also
-- legitimately exist if the tenant runs both Zoom AND Teams and the
-- person attended meetings on both — in that case the join key
-- (tenant, person_key, date) is the same but `data_source` differs.
--
-- The test therefore allows up-to-one row per `data_source` per
-- (tenant, person_key, date), but flags >1 row for the same data_source.

SELECT
    tenant_id,
    person_key,
    date,
    data_source,
    count() AS n,
    groupUniqArray(insight_source_id) AS source_ids
FROM silver.class_collab_meeting_activity FINAL
WHERE person_key IS NOT NULL AND person_key != ''
GROUP BY tenant_id, person_key, date, data_source
HAVING n > 1
LIMIT 100
