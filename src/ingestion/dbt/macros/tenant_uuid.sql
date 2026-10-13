{#-
  The canonical tenant UUID a Bronze row is stamped with, typed. Fails the
  build on a value that is not a UUID: Bronze still carrying a legacy tenant
  key means the tenant migration has not run (deploy/TENANT_IDENTITY_UPGRADE.md).
-#}
{% macro tenant_uuid(tenant_col) -%}
    toUUID(coalesce({{ tenant_col }}, ''))
{%- endmacro %}
