{{ config(
    severity='warn',
    tags=['connector_quality', 'allure'],
    store_failures=true,
    meta={
        'title': 'Every Allure launch environment row names a launch the launches stream synced',
        'domain': 'quality',
        'category': 'completeness',
        'tier': 'warn',
        'remediation': 'Environment rows extracted more than 2 days ago name a launch_id that bronze_allure.launches does not hold. The `launches` stream pages separately from the `_launches` parent of `launch_environment`, and a launch deleted mid-sync from a page already read shifts the next page by one row, so one stream can skip a launch the other read. Look the launch up in Allure TestOps. If it still exists, reset the `launches` stream so the next sync re-reads the last 90 days. If it was deleted, the rows belong to a deleted launch and nothing is lost.'
    }
) }}

SELECT
    e.tenant_id AS tenant_id,
    e.source_id AS source_id,
    e.launch_id AS launch_id,
    count() AS environment_rows,
    max(e._airbyte_extracted_at) AS last_extracted_at
FROM {{ source('bronze_allure', 'launch_environment') }} AS e FINAL
LEFT ANTI JOIN (
    SELECT DISTINCT tenant_id, source_id, id
    FROM {{ source('bronze_allure', 'launches') }} FINAL
) AS l
    ON l.tenant_id = e.tenant_id AND l.source_id = e.source_id AND l.id = e.launch_id
WHERE e._airbyte_extracted_at < now64(3) - INTERVAL 2 DAY
GROUP BY e.tenant_id, e.source_id, e.launch_id
LIMIT 100
