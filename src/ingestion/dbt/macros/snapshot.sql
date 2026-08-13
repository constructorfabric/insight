{% macro snapshot(source_ref, unique_key_col, check_cols, check_raw_data_cols=[], check_raw_data_all=false, raw_data_exclude_keys=[]) %}
{#
  Incremental append-only SCD2 snapshot.
  Appends a new row only when tracked columns change.

  Args:
    source_ref:           source() or ref() to the raw table
    unique_key_col:       column that uniquely identifies an entity
    check_cols:           list of top-level columns to monitor for changes
    check_raw_data_cols:  list of field names inside `raw_data` JSON column to monitor
                          (extracted via JSONExtractString; missing keys yield '')
    check_raw_data_all:   monitor every key of `raw_data`, whatever the source emits
    raw_data_exclude_keys: keys check_raw_data_all leaves out

  Adds columns:
    _row_hash    — cityHash64 of tracked columns (for comparison)
    _tracked_at  — timestamp when the version was captured
#}

WITH source_data AS (
    SELECT
        *,
        cityHash64(
            {% for col in check_cols %}
            ifNull(toString({{ col }}), '__null__'),
            {% endfor %}
            {% for col in check_raw_data_cols %}
            JSONExtractString(ifNull(toString(raw_data), '{}'), '{{ col }}'){{ ',' if not loop.last }}
            {% endfor %}
            {% if not check_raw_data_cols %}
            ''
            {% endif %}
            {% if check_raw_data_all %}
            -- mapSort makes the hash independent of the key order the source
            -- happened to serialise raw_data in.
            , toString(mapSort({{ raw_data_fields(raw_data_exclude_keys) }}))
            {% endif %}
        ) AS _row_hash
    -- FINAL dedups the ReplacingMergeTree source to one row per key (latest
    -- version) BEFORE hashing. Without it, transient pre-merge duplicates
    -- (e.g. an erroneous Airbyte full_refresh|append re-appending every row)
    -- are each compared to the snapshot high-water mark and written as spurious
    -- SCD2 history versions — data corruption, not just dupes. See ADR-0001.
    -- Every source_ref MUST therefore be a ReplacingMergeTree relation (bronze is
    -- promoted; intermediate models like slack__users_latest are RMT too).
    FROM {{ source_ref }} FINAL
)

{% if is_incremental() %}

{#- The adapter's INSERT maps SELECT output to the persisted table's columns
    POSITIONALLY. A `s.*` projection follows the source's physical column
    order, so a source rebuilt with a different layout (e.g. an Airbyte
    connector migration) silently lands values in unrelated columns. Project
    explicitly in the persisted target's own order, and refuse to write when
    the column sets have drifted apart. -#}
{%- set target_columns = adapter.get_columns_in_relation(this) -%}
{%- set source_names = adapter.get_columns_in_relation(source_ref) | map(attribute='name') | list -%}
{%- set generated_names = ['_row_hash', '_tracked_at'] -%}
{%- set target_names = target_columns | map(attribute='name') | list -%}
{%- set missing_in_source = target_names | reject('in', source_names) | reject('in', generated_names) | list -%}
{%- set missing_in_target = source_names | reject('in', target_names) | list -%}
{%- if missing_in_source or missing_in_target -%}
    {{ exceptions.raise_compiler_error(
        'snapshot(): column drift between ' ~ source_ref ~ ' and persisted ' ~ this ~ '. ' ~
        'Missing in source: [' ~ missing_in_source | join(', ') ~ ']. ' ~
        'New in source: [' ~ missing_in_target | join(', ') ~ ']. ' ~
        'Migrate or rebuild the snapshot table before appending — a blind append would corrupt it.'
    ) }}
{%- endif %}

, latest AS (
    SELECT
        {{ unique_key_col }},
        argMax(_row_hash, _tracked_at) AS _row_hash
    FROM {{ this }}
    GROUP BY {{ unique_key_col }}
)

SELECT
    {%- for col in target_columns %}
    {% if col.name == '_tracked_at' -%}
        now() AS _tracked_at
    {%- elif col.name == '_row_hash' -%}
        s._row_hash
    {%- else -%}
        s.{{ adapter.quote(col.name) }}
    {%- endif %}{{ ',' if not loop.last }}
    {%- endfor %}
FROM source_data s
LEFT JOIN latest l ON s.{{ unique_key_col }} = l.{{ unique_key_col }}
WHERE l.{{ unique_key_col }} IS NULL
   OR s._row_hash != l._row_hash

{% else %}

SELECT
    *,
    now() AS _tracked_at
FROM source_data

{% endif %}

{% endmacro %}
