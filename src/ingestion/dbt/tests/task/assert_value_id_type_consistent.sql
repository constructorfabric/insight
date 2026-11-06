{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'A field keeps one value id type',
        'domain': 'task-tracking',
        'category': 'consistency',
        'tier': 'warn',
        'remediation': 'One field carries rows with different value_id_type. Changing a custom field\'s type in the tracker leaves the older rows with the old type; otherwise the producer classifies the field inconsistently.'
    }
) }}

-- value_id_type must be stable per (tenant_id, insight_source_id, data_source, field_id). A field's
-- identifier kind doesn't change within one source instance — if we observe two different
-- value_id_types for the same field_id, the classifier is non-deterministic or the field
-- metadata diverged across runs. Jira `customfield_NNNNN` IDs are instance-local, so two
-- Jira tenants can legitimately have different types for the same field_id — scope by
-- (`tenant_id`, `insight_source_id`) to avoid false positives across sources.

SELECT
    tenant_id,
    insight_source_id,
    data_source,
    field_id,
    groupArray(DISTINCT value_id_type) AS observed_types
FROM silver.class_task_field_history FINAL
GROUP BY tenant_id, insight_source_id, data_source, field_id
HAVING length(observed_types) > 1
