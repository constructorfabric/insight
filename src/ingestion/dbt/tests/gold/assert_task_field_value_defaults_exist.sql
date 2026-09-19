{{ config(
    severity='error',
    store_failures=true,
    meta={
        'title': 'Every standardized task field has a default decision',
        'domain': 'task',
        'category': 'coverage',
        'tier': 'error',
        'remediation': "A (tenant, source, field) the pipeline standardizes has no live config.field_value_defaults row, so an unmapped key falls to gold's hardcoded 'unknown' terminal — a classification nobody decided. Author the row in environments/<env>/clickhouse/field-value-map/*.tsv (insert-only, loaded on deploy) and redeploy. A tenant that genuinely wants unclassified keys counted as unknown records that as the decision, with default_value = 'unknown'. A row whose default_value is outside the field's domain counts as missing here; assert_field_values_are_canonical reports the value itself."
    }
) }}
-- A build-integrity test, untagged: it guards seeded and fixture environments,
-- where a missing row is a bug. In a deployed install the signal is the Grafana
-- rule, since the 'unknown' terminal in task_issue_state makes the metrics read
-- the same either way.
WITH
live_sources AS (
    SELECT DISTINCT
        tenant_id,
        insight_source_id
    FROM {{ ref('class_task_users') }} FINAL
),
-- A field is standardized for a source only where the role is bound: GitHub
-- carries no resolution binding, and demanding its default would be noise.
standardized AS (
    SELECT DISTINCT
        s.tenant_id         AS tenant_id,
        s.insight_source_id AS insight_source_id,
        r.data_source       AS data_source,
        roles.2             AS field
    FROM live_sources AS s
    INNER JOIN {{ ref('task_field_roles_current') }} AS r
        ON r.insight_source_id = s.insight_source_id
    ARRAY JOIN [
        tuple('issuetype', 'issue_type'),
        tuple('resolution', 'resolution')
    ] AS roles
    WHERE r.role = roles.1
),
-- Same bitemporal shape as task_issue_state's default CTEs: latest row per key
-- first, tombstone and domain filter after, so a retraction cannot resurrect
-- the decision it retracts.
decided AS (
    SELECT
        tenant_id,
        insight_source_id,
        field
    FROM (
        SELECT
            tenant_id,
            insight_source_id,
            field,
            default_value,
            is_deleted
        FROM {{ source('config', 'field_value_defaults') }} FINAL
        WHERE field IN ('issue_type', 'resolution')
          AND valid_from <= now64(3)
        ORDER BY valid_from DESC, recorded_at DESC
        LIMIT 1 BY tenant_id, insight_source_id, field
    )
    WHERE is_deleted = 0
      AND ((field = 'issue_type' AND default_value IN ('bug', 'task', 'unknown'))
           OR (field = 'resolution' AND default_value IN ('fixed', 'duplicate', 'wontfix', 'unknown')))
)
SELECT
    u.tenant_id         AS tenant_id,
    u.insight_source_id AS insight_source_id,
    u.data_source       AS data_source,
    u.field             AS field
FROM standardized AS u
LEFT ANTI JOIN decided AS d
    ON d.tenant_id = u.tenant_id
    AND d.insight_source_id = u.insight_source_id
    AND d.field = u.field
