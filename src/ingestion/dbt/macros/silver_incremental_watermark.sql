{#-
  silver_incremental_watermark(source_keys, alias='candidate')

  The incremental boundary for a silver class table, scoped to ONE source
  instance instead of the whole table.

  A class table is written by every connector that feeds it, and each connector
  runs on its own schedule. Producers stamp `_version` from their own clock —
  extraction time, snapshot time, or the source record's own update time — so
  the values of two producers are not comparable. A single
  `WHERE _version > (SELECT max(_version) FROM this)` therefore lets whichever
  producer commits first raise the boundary above another producer's rows, and
  those rows are then below it FOREVER: they never reach silver, and nothing
  reports the loss. Per-instance scoping is the same fix already applied to the
  crm classes and to class_person_attribute_claims.

  `coalesce(max_version, 0)` is load-bearing: a source instance absent from the
  target has no watermark row, and comparing against NULL would drop every row
  of every new instance — reintroducing the bug it exists to prevent.

  Emits nothing on a full refresh or a first run, so the model reads its whole
  union. Re-reading a row is safe: silver is delete+insert on `unique_key`.
-#}
{% macro silver_incremental_watermark(source_keys, alias='candidate') %}
{%- if is_incremental() %}
LEFT JOIN (
    SELECT
        {{ source_keys | join(',\n        ') }},
        max(_version) AS max_version
    FROM {{ this }}
    GROUP BY
        {{ source_keys | join(',\n        ') }}
) AS watermarks
    ON {% for key in source_keys %}{% if not loop.first %}   AND {% endif %}{{ alias }}.{{ key }} = watermarks.{{ key }}
    {% endfor %}
WHERE {{ alias }}._version > coalesce(watermarks.max_version, 0)
{%- endif %}
{% endmacro %}
