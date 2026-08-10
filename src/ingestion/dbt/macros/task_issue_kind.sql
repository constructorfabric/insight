{# Maps an issue-type name to `issue_kind` (bug / other / unknown) against the
   configured name lists. Feed it the most stable name the source has (Jira
   `untranslatedName`), not a display name. #}

{% macro task_type_name_array(names) -%}
[{% for name in names %}'{{ name | lower | replace("'", "''") }}'{{ ", " if not loop.last }}{% endfor %}]
{%- endmacro %}

{% macro task_issue_kind(name_expr) %}
{%- set normalized = "lower(trimBoth(ifNull(" ~ name_expr ~ ", '')))" -%}
multiIf(
    has({{ task_type_name_array(var('task_bug_type_names')) }}, {{ normalized }}), 'bug',
    has({{ task_type_name_array(var('task_non_bug_type_names')) }}, {{ normalized }}), 'other',
    'unknown'
)
{% endmacro %}
