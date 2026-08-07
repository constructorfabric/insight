{% macro raw_data_fields(exclude_keys=[]) %}
{#
  Every key of the `raw_data` JSON column as a Map(String, String), minus
  exclude_keys.

  Args:
    exclude_keys: field names to leave out of the map

  Values are coerced exactly as JSONExtractString coerces them, so a field read
  by name and the same field read out of this map compare equal — change
  detection and the field-level history must not disagree about a value.
#}

{%- set pairs = "JSONExtractKeysAndValues(ifNull(toString(raw_data), '{}'), 'String')" -%}

{%- if exclude_keys -%}
{%- set quoted = [] -%}
{%- for key in exclude_keys -%}
{%- do quoted.append("'" ~ key ~ "'") -%}
{%- endfor -%}
{%- set pairs = "arrayFilter(x -> x.1 NOT IN (" ~ quoted | join(", ") ~ "), " ~ pairs ~ ")" -%}
{%- endif -%}

CAST({{ pairs }}, 'Map(String, String)')
{%- endmacro %}
