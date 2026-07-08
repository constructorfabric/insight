-- Build-integrity check (untagged → error severity under `dbt build`).
-- git_metric_observations LEFT JOINs identity.git_actor_emails on
-- (tenant_id, data_source, actor_name) to resolve pull-request authors;
-- more than one row per key would fan out PR observations and inflate
-- every PR-derived measure. Any returned row is a violation.
SELECT
    tenant_id,
    data_source,
    actor_name,
    count() AS row_count
FROM {{ ref('git_actor_emails') }}
GROUP BY tenant_id, data_source, actor_name
HAVING count() > 1
