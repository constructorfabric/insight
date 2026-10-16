{#-
  The canonical tenant UUID a Bronze row is stamped with, typed. A value that
  is not a UUID fails the build: Bronze has not been moved to the canonical
  tenant yet, and no row may resolve under a tenant nobody signs in to.
-#}
{% macro tenant_uuid(tenant_col) -%}
    toUUID(coalesce({{ tenant_col }}, ''))
{%- endmacro %}
