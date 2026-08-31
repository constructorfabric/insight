-- Build-integrity check (untagged → error severity under `dbt build`).
-- One row per deployment: the status events fold to one outcome before the
-- join, so a second row means a deployment gained an outcome per status event
-- and the deployment count is inflated.
SELECT
    tenant_id,
    source_id,
    deployment_id,
    count() AS row_count
FROM {{ ref('ci_deployments') }}
GROUP BY tenant_id, source_id, deployment_id
HAVING count() > 1
