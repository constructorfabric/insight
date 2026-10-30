{% test not_empty(model, column_name) %}
SELECT {{ column_name }}
FROM {{ model }}
WHERE {{ column_name }} = ''
{% endtest %}
