{{ config(
    tags=['data_quality'],
    severity='warn',
    store_failures=true,
    meta={
        'title': 'every AI staging row reaches its silver class',
        'domain': 'ai',
        'category': 'completeness',
        'tier': 'error',
        'remediation': 'A staging row a silver class never admitted is lost for good — nothing re-reads it. The usual cause is an incremental boundary that is not scoped to the source instance, so one connector''s run raised the boundary past another connector''s rows: check the class model uses silver_incremental_watermark rather than a table-wide max(_version). The rows are still in staging, so recovery is a re-read of the union — a full refresh of the class, or an anti-join insert from staging.'
    }
) }}

{#- One row per (class, source instance) whose staging rows did not arrive.
    Classes whose contributors are not materialised here are skipped rather than
    reported empty: this deployment does not run them. -#}
{%- set ai_classes = [
    ('silver:class_ai_dev_usage', 'class_ai_dev_usage'),
    ('silver:class_ai_assistant_usage', 'class_ai_assistant_usage'),
    ('silver:class_ai_overage', 'class_ai_overage')
] -%}
{%- set branches = [] -%}
{%- for tag_name, class_model in ai_classes -%}
  {%- set staged_models = materialised_models_for_tag(tag_name) -%}
  {%- if staged_models | length > 0 -%}
    {%- do branches.append((class_model, staged_models)) -%}
  {%- endif -%}
{%- endfor -%}

{% if branches | length == 0 %}
SELECT
    CAST('' AS String)              AS class_name,
    CAST(NULL AS Nullable(String))  AS source_id,
    toUInt64(0)                     AS missing_rows
WHERE 1 = 0
{% else %}
{% for class_model, staged_models in branches %}
SELECT
    '{{ class_model }}'         AS class_name,
    staged.source_id            AS source_id,
    count()                     AS missing_rows
FROM (
    {% for staged_model in staged_models %}
    SELECT DISTINCT unique_key, source_id FROM {{ ref(staged_model) }}
    {%- if not loop.last %}
    UNION ALL
    {%- endif %}
    {% endfor %}
) AS staged
LEFT ANTI JOIN {{ ref(class_model) }} AS served USING (unique_key)
GROUP BY class_name, source_id
{% if not loop.last %}UNION ALL{% endif %}
{% endfor %}
{% endif %}
