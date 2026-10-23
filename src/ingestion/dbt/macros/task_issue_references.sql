{#-
  Issue keys written in text, for `silver.class_task_references`. A key is a
  project key — an uppercase letter, then uppercase letters, digits or `_` —
  a hyphen and a number, standing as a whole word. The shape alone admits
  `UTF-8`, so a producer keeps only keys that resolve to an issue or carry a
  synced project's prefix.
-#}


{#- Every key-shaped token of a plain text, in order of appearance. -#}
{% macro task_issue_keys(text) %}
    extractAll({{ text }}, '(?:^|[^A-Za-z0-9_])([A-Z][A-Z0-9_]+-[0-9]+)\\b')
{% endmacro %}
