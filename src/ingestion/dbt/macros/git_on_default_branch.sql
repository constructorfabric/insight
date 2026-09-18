{# Whether a repository's copy of a commit sits on that repository's default
   branch at sync time. An unreported flag reads as not landed, and the result
   is never NULL, so callers can sort and OR on it. #}
{% macro git_on_default_branch(flag) -%}
coalesce({{ flag }}, 0) = 1
{%- endmacro %}
