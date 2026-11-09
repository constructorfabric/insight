{#-
  instance_incremental_watermark(column, row_keys=none, this_keys=['tenant_id', 'source_id'], lookback=none)

  The incremental boundary of a model for the source instance of the
  row being filtered: `max(column)` over that instance's rows in `this`, less
  `lookback`. An expression, so it replaces a table-wide
  `(SELECT max(column) FROM {{ this }})` in place:

      WHERE _airbyte_extracted_at > {{ instance_incremental_watermark('_airbyte_extracted_at') }}

  Two instances of one connector share a table and sync on their own
  schedules. A boundary over the whole table lets one instance raise it above
  the other's rows, which `append` then never revisits (#752).

  An instance `this` does not hold yet gets the type's zero (the epoch), so its
  every row passes — no `count() = 0` guard is needed. `lookback` is applied
  inside, to real values only, so a Date floor cannot wrap below 1970.

  `this_keys` name the instance in `this`; `row_keys` name it in the row being
  filtered, qualified when the query joins. Keys compare NULL-safely, as in
  silver_incremental_watermark.
-#}
{% macro instance_incremental_watermark(column, row_keys=none, this_keys=['tenant_id', 'source_id'], lookback=none) %}
(
    SELECT mapFromArrays(groupArray(instance), groupArray(watermark))
    FROM (
        SELECT
            {{ _instance_key(this_keys) }} AS instance,
            max(assumeNotNull({{ column }})){% if lookback %} - INTERVAL {{ lookback }}{% endif %} AS watermark
        FROM {{ this }}
        WHERE {{ column }} IS NOT NULL
        GROUP BY instance
    )
)[{{ _instance_key(row_keys or this_keys) }}]
{%- endmacro %}

{% macro _instance_key(keys) -%}
concat({% for key in keys %}{% if not loop.first %}, '\x1f', {% endif %}ifNull(toString({{ key }}), ''){% endfor %})
{%- endmacro %}
