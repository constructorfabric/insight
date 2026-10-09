{#-
  The tenant segment of a key a model builds itself, for models whose rows
  outlive a full refresh (tag:history) or feed one.

  A warm install keeps its legacy tenant key in every stored key while
  tenant_id moves to the canonical UUID (deploy/TENANT_IDENTITY_UPGRADE.md).
  Rows of the canonical tenant map back to that key, so a key built after the
  migration matches the one built before it. Without INSIGHT_LEGACY_TENANT_KEY
  this is the tenant column unchanged.
-#}
{% macro tenant_key(tenant_col) -%}
    {%- set canonical = env_var('INSIGHT_TENANT_ID', '') -%}
    {%- set legacy = env_var('INSIGHT_LEGACY_TENANT_KEY', '') -%}
    {%- if canonical and legacy -%}
    if(coalesce({{ tenant_col }}, '') = '{{ canonical }}', '{{ legacy }}', coalesce({{ tenant_col }}, ''))
    {%- else -%}
    coalesce({{ tenant_col }}, '')
    {%- endif -%}
{%- endmacro %}
