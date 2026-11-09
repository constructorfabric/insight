{#-
  The canonical tenant UUID a Bronze row is stamped with, typed. A missing
  tenant or a value that is not a UUID fails the build: no row may resolve
  under a tenant nobody signs in to.
-#}
{% macro tenant_uuid(tenant_col) -%}
    toUUID(ifNull({{ tenant_col }}, ''))
{%- endmacro %}
