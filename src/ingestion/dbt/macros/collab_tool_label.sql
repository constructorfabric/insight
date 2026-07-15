{# Display label for the collaboration `tool` dimension. Static product
   vocabulary (same rationale as the git source label), centralized so the
   gold view carries no inline vendor mapping. Values are the `data_source`
   discriminator with the `insight_` prefix stripped.

   `m365_label` names the M365 surface a measure actually reads: chat and
   meeting measures come from the Teams activity report, so those call
   sites pass 'Microsoft Teams'; the suite-level default fits email and
   files (Outlook, OneDrive + SharePoint). The dimension VALUE stays
   `m365` either way — one platform identity for colors, filters, and the
   breadth count; only the display label is surface-specific. #}

{% macro collab_tool_label(tool_expr, m365_label='Microsoft 365') %}
multiIf(
    {{ tool_expr }} = 'm365', '{{ m365_label }}',
    {{ tool_expr }} = 'slack', 'Slack',
    {{ tool_expr }} = 'zoom', 'Zoom',
    {{ tool_expr }} = 'zulip_proxy', 'Zulip',
    {{ tool_expr }}
)
{% endmacro %}
