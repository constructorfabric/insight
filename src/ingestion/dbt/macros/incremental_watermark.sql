{% macro incremental_watermark(src, tenant_col='tenant_id', source_col='source_id', version_col='_version') %}
{#-
  Per-(tenant, source) incremental high-watermark.  Fixes #752.

  The previous pattern used a GLOBAL watermark:
      WHERE _version > (SELECT max(_version) FROM {{ this }})
  In a multi-tenant table this silently drops rows for any (tenant, source)
  whose own max(_version) is below the global max — i.e. every late-joining
  tenant, every paused-then-resumed source, every historical backfill.
  No error, no log: the rows just never land in silver.

  This macro filters each incoming row against the high-watermark of ITS OWN
  (tenant, source) instead of the table-wide maximum.

  Implemented as a LEFT JOIN (not a correlated subquery — ClickHouse does not
  support correlated subqueries in WHERE).

  Args:
    src         SQL that produces the rows (e.g. a `union_by_tag(...)` call)
    tenant_col  tenant identity column in `src`  (default 'tenant_id')
    source_col  source identity column in `src`  (default 'source_id')
    version_col incremental version column       (default '_version')
-#}
{% if is_incremental() %}
WITH _wm_src AS (
    {{ src }}
)
SELECT _wm_src.*
FROM _wm_src
LEFT JOIN (
    SELECT {{ tenant_col }} AS _wm_t, {{ source_col }} AS _wm_s,
           max({{ version_col }}) AS _wm_hwm, 1 AS _wm_seen
    FROM {{ this }}
    GROUP BY {{ tenant_col }}, {{ source_col }}
) _wm
    ON _wm._wm_t = _wm_src.{{ tenant_col }}
   AND _wm._wm_s = _wm_src.{{ source_col }}
-- _wm_seen = 0 means this (tenant, source) has never been loaded → keep ALL its
-- rows, including a legitimate {{ version_col }} = 0 first row. NB: ClickHouse fills
-- unmatched LEFT JOIN columns with type DEFAULTS (0), not NULL (unless
-- join_use_nulls=1), so an `IS NULL` guard would NOT work here — hence the
-- explicit seen-flag.
WHERE _wm._wm_seen = 0
   OR _wm_src.{{ version_col }} > _wm._wm_hwm
{% else %}
SELECT * FROM (
    {{ src }}
)
{% endif %}
{% endmacro %}
