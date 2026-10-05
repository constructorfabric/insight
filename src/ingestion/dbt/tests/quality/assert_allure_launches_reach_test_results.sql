{{ config(
    severity='warn',
    tags=['connector_quality', 'allure'],
    store_failures=true,
    meta={
        'title': 'Every closed Allure launch past the lookback has test results',
        'domain': 'quality',
        'category': 'completeness',
        'tier': 'warn',
        'remediation': 'A closed launch whose lastModifiedDate is more than 2 days old has no row in bronze_allure.test_results, and no later sync re-reads it unless the launch changes. Open the launch in Allure TestOps. If it holds no results (an upload that failed or was cancelled), the source is empty and nothing is lost. If it holds results, the `_launches` parent of `test_results` skipped it: a launch deleted mid-sync from a page already read shifts the next page by one row. Reset the `test_results` stream so the next sync re-reads every launch of the last 90 days. If every launch fires, `test_results` is not selected in the connection.'
    }
) }}

SELECT
    l.tenant_id AS tenant_id,
    l.source_id AS source_id,
    l.projectId AS project_id,
    l.id AS launch_id,
    fromUnixTimestamp64Milli(l.lastModifiedDate, 'UTC') AS launch_modified_at
FROM {{ source('bronze_allure', 'launches') }} AS l FINAL
LEFT ANTI JOIN (
    SELECT DISTINCT tenant_id, source_id, launchId
    FROM {{ source('bronze_allure', 'test_results') }} FINAL
) AS r
    ON r.tenant_id = l.tenant_id AND r.source_id = l.source_id AND r.launchId = l.id
WHERE l.closed
    AND l.lastModifiedDate < toUnixTimestamp64Milli(now64(3)) - 2 * 86400000
LIMIT 100
