{# Display label for the collaboration `tool` dimension. Static product
   vocabulary (same rationale as the git source label), centralized so the
   gold view carries no inline vendor mapping. Values are the `data_source`
   discriminator with the `insight_` prefix stripped. #}

{% macro collab_tool_label(tool_expr) %}
multiIf(
    {{ tool_expr }} = 'm365', 'Microsoft 365',
    {{ tool_expr }} = 'slack', 'Slack',
    {{ tool_expr }} = 'zoom', 'Zoom',
    {{ tool_expr }} = 'zulip_proxy', 'Zulip',
    {{ tool_expr }}
)
{% endmacro %}
