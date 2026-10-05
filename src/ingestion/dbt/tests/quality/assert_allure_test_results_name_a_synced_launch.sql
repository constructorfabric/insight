{{ config(
    severity='warn',
    tags=['connector_quality', 'allure'],
    store_failures=true,
    meta={
        'title': 'Every Allure test result names a launch the launches stream synced',
        'domain': 'quality',
        'category': 'completeness',
        'tier': 'warn',
        'remediation': 'Test results extracted more than 2 days ago name a launchId that bronze_allure.launches does not hold. The `launches` stream pages separately from the `_launches` parent of `test_results`, and a launch deleted mid-sync from a page already read shifts the next page by one row, so one stream can skip a launch the other read. Look the launch up in Allure TestOps. If it still exists, reset the `launches` stream so the next sync re-reads the last 90 days. If it was deleted, the results are orphans of a deleted launch and nothing is lost. After a `launches` reset, results of launches older than 90 days fire too, because no sync re-reads those launches.'
    }
) }}

SELECT
    r.tenant_id AS tenant_id,
    r.source_id AS source_id,
    r.launchId AS launch_id,
    count() AS test_results,
    max(r._airbyte_extracted_at) AS last_extracted_at
FROM {{ source('bronze_allure', 'test_results') }} AS r FINAL
LEFT ANTI JOIN (
    SELECT DISTINCT tenant_id, source_id, id
    FROM {{ source('bronze_allure', 'launches') }} FINAL
) AS l
    ON l.tenant_id = r.tenant_id AND l.source_id = r.source_id AND l.id = r.launchId
WHERE r._airbyte_extracted_at < now64(3) - INTERVAL 2 DAY
GROUP BY r.tenant_id, r.source_id, r.launchId
LIMIT 100
