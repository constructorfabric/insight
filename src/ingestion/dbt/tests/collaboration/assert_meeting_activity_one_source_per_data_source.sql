-- A tenant should have AT MOST ONE `insight_source_id` per `data_source`
-- in `silver.class_collab_meeting_activity`. More than one almost always
-- means a parallel / duplicate Airbyte source slipped through — see
-- issue #283 for the canonical case (a tenant with two Zoom sources
-- `main` and `zoom-main` running in parallel, doubling reported hours).
--
-- This is a deployment-shape contract, not a hard SQL invariant. The
-- legitimate case for multiple sources per data_source is uncommon
-- (e.g. multi-org Slack would use one source per workspace, but
-- collab_meeting_activity is a single-org metric). If a real use case
-- emerges, narrow the test to known-bad source_id values rather than
-- relaxing the rule globally.
--
-- Failure rows show which (tenant, data_source) combination has the
-- problem and which source_ids are competing.

SELECT
    tenant_id,
    data_source,
    count() AS distinct_source_ids,
    groupUniqArray(insight_source_id) AS source_ids
FROM (
    SELECT DISTINCT tenant_id, data_source, insight_source_id
    FROM silver.class_collab_meeting_activity FINAL
    WHERE insight_source_id IS NOT NULL AND insight_source_id != ''
)
GROUP BY tenant_id, data_source
HAVING distinct_source_ids > 1
LIMIT 100
