{#- Equal exactly when two value lists hold the same values with the same multiplicity, in any order,
    so a join can hash one integer per side instead of carrying the arrays. Callers pass lists already
    deduplicated by id, so multiplicity only survives where it is real: two attachments that share a
    filename are two displays, not one. -#}
{% macro jira_value_multiset_digest(values) -%}
cityHash64(arraySort({{ values }}))
{%- endmacro %}
