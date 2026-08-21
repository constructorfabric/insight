{% set evidence_models = [
    'ai_cost_metric_evidence',
    'ai_metric_evidence',
    'collab_metric_evidence',
    'git_metric_evidence',
    'task_metric_evidence',
    'wiki_metric_evidence'
] %}

{% for model in evidence_models %}
SELECT
    '{{ model }}' AS evidence_model,
    tenant_id,
    source_key,
    measure_key,
    entity_id,
    metric_date,
    record_id,
    count() AS row_count
FROM {{ ref(model) }}
GROUP BY tenant_id, source_key, measure_key, entity_id, metric_date, record_id
HAVING count() > 1
{% if not loop.last %}
UNION ALL
{% endif %}
{% endfor %}
