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

{# One row per source row stamped with the subject to count, feeding
   distinct-count metrics (`uniqExact(subject_key)` at query time). `value` is
   a constant 1 so the same measure also sums to a per-group row count where a
   ratio binds it as a denominator; `subject_key` carries the counted subject
   (a date for active days, a tool for breadth). Callers MUST dedup the source
   relation to one row per intended subject — this macro does not group.
   Carries the `subject_key` column the other measure macros omit, so
   distinct-count measures live in their own UNION branch. #}
{% macro distinct_measure(measure_key, relation, subject_key_expr, dimensions_col) %}
    SELECT
        tenant_id,
        entity_id,
        metric_date,
        '{{ measure_key }}' AS measure_key,
        toNullable(toFloat64(1)) AS value,
        toNullable(toString({{ subject_key_expr }})) AS subject_key,
        {{ dimensions_col }} AS dimensions
    FROM {{ relation }}
    WHERE ({{ subject_key_expr }}) IS NOT NULL
{% endmacro %}
