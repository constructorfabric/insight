{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'AI class label columns are non-empty',
        'domain': 'ai',
        'category': 'contract',
        'tier': 'error',
        'remediation': 'The class contract guarantees connector-declared non-empty tool_label / surface_label — insight.ai_metric_observations builds dimension tuples from them verbatim, with no downstream fallback. An empty label means either a staging model stopped declaring it (fix the staging model) or historical rows predate the column and the label backfill migration (20260707000000_ai_class_label_backfill.sql) has not been applied.'
    }
) }}

SELECT
    'class_ai_dev_usage' AS relation,
    insight_tenant_id,
    tool AS discriminator,
    day
FROM {{ ref('class_ai_dev_usage') }}
WHERE tool_label = ''

UNION ALL

SELECT
    'class_ai_assistant_usage' AS relation,
    insight_tenant_id,
    concat(tool, '/', surface) AS discriminator,
    day
FROM {{ ref('class_ai_assistant_usage') }}
WHERE tool_label = '' OR surface_label = ''
