-- Build-integrity check (untagged → error severity under `dbt build`).
-- A tenant-grain dataset keys every row by the tenant itself. The semantic
-- compiler groups these relations by entity_id and never joins an identity
-- pool, so an entity_id that is not the tenant id would split one tenant's
-- value into rows no caller can ask for.
{% set tenant_grain_models = ['ci_runs', 'ci_deployments', 'ci_commits'] %}

{% for model in tenant_grain_models %}
SELECT
    '{{ model }}' AS dataset,
    tenant_id,
    entity_id,
    count() AS row_count
FROM {{ ref(model) }}
WHERE entity_id != tenant_id
GROUP BY tenant_id, entity_id
{% if not loop.last %}
UNION ALL
{% endif %}
{% endfor %}
