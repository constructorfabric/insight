{% macro sum_measure(measure_key, relation, value_expr, dimensions_col, where=none) %}
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        '{{ measure_key }}' AS measure_key,
        toNullable(sumIf(toFloat64({{ value_expr }}), ({{ value_expr }}) IS NOT NULL)) AS value,
        {{ dimensions_col }} AS dimensions
    FROM {{ relation }}
    {% if where %}WHERE {{ where }}
    {% endif %}GROUP BY tenant_id, entity_id, metric_date, {{ dimensions_col }}
    HAVING countIf(({{ value_expr }}) IS NOT NULL) > 0
{% endmacro %}

{# One row per source event, no aggregation: the observation shape for
   median-computation metrics, which aggregate over events at query time.
   Multiple rows per (entity, day, measure) are the intended grain; only
   median/percentile-style metrics may bind these measures. #}
{% macro event_measure(measure_key, relation, value_expr, dimensions_col, where=none) %}
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        '{{ measure_key }}' AS measure_key,
        toNullable(toFloat64({{ value_expr }})) AS value,
        {{ dimensions_col }} AS dimensions
    FROM {{ relation }}
    WHERE ({{ value_expr }}) IS NOT NULL
    {% if where %}  AND ({{ where }})
    {% endif %}
{% endmacro %}

{% macro presence_measure(measure_key, relations) %}
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        '{{ measure_key }}' AS measure_key,
        toNullable(toFloat64(1)) AS value,
        CAST([] AS Array(Tuple(key String, value String, label Nullable(String)))) AS dimensions
    FROM (
        {%- for relation in relations %}
        SELECT DISTINCT
            tenant_id,
            entity_id,
            metric_date
        FROM {{ relation }}
        {%- if not loop.last %}
        UNION DISTINCT
        {%- endif %}
        {%- endfor %}
    )
{% endmacro %}
