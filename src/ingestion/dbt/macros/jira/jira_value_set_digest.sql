{#- Equal exactly when two value sets hold the same distinct values in any order,
    so a join can hash one integer per side instead of carrying the arrays. -#}
{% macro jira_value_set_digest(values) -%}
cityHash64(arraySort(arrayDistinct({{ values }})))
{%- endmacro %}
