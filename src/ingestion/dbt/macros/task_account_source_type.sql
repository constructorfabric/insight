{#-
  A task class row's `data_source` in identity's `source_type` vocabulary — the
  value the connector's identity_inputs stamp on its account bindings. '' for a
  tracker with no account bindings keeps resolution on the email path.
-#}

{% macro task_account_source_type(data_source_expr) %}
    multiIf(
        {{ data_source_expr }} = 'jira', 'jira',
        {{ data_source_expr }} = 'youtrack', 'youtrack',
        {{ data_source_expr }} = 'github', 'github',
        ''
    )
{% endmacro %}
