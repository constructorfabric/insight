{#-
  Shared expressions for the HubSpot staging models. Every model reads Bronze
  tables written by the same CDK connector, so the envelope columns
  (`raw_data`, `archived`, `archivedAt`, `updatedAt`) and the incremental
  boundary are identical across them.
-#}

{% macro hubspot_bronze_tables(live_table, archived=true) %}
  {#- The live Bronze table plus its `_archived` sibling once it exists. -#}
  {%- set tables = [live_table] -%}
  {%- if archived -%}
    {%- set bronze_schema = source('bronze_hubspot', live_table).schema -%}
    {%- if adapter.get_relation(database=none, schema=bronze_schema, identifier=live_table ~ '_archived') -%}
      {%- do tables.append(live_table ~ '_archived') -%}
    {%- endif -%}
  {%- endif -%}
  {{ return(tables) }}
{% endmacro %}


{% macro hubspot_properties_map() -%}
  {#- Every non-empty HubSpot property of the record, standard and custom. -#}
  CAST(
      arrayFilter(
          kv -> kv.2 != '',
          JSONExtractKeysAndValues(ifNull(JSONExtractRaw(ifNull(raw_data, '{}'), 'properties'), '{}'), 'String')
      ),
      'Map(String, String)'
  )
{%- endmacro %}


{% macro hubspot_record_version() -%}
  {#- An archive event outranks the live update it follows. -#}
  greatest(
      coalesce(toUnixTimestamp64Milli(updatedAt), 0),
      coalesce(toUnixTimestamp64Milli(archivedAt), 0)
  )
{%- endmacro %}


{% macro hubspot_deletion_columns() -%}
  toUInt8(coalesce(archived, false))              AS is_deleted,
        archivedAt                                      AS deleted_at
{%- endmacro %}


{% macro hubspot_first_association(column) -%}
  nullIf(arrayElement(JSONExtract(coalesce({{ column }}, '[]'), 'Array(String)'), 1), '')
{%- endmacro %}


{% macro hubspot_staging_tail(cte) %}
{#-
  One row per unique_key at a version this table does not hold yet: a batch
  can carry several versions of a record (unmerged Bronze parts, a live and an
  archived arm), and silver must not inherit them.
-#}
SELECT {{ cte }}.*
FROM {{ cte }}
{% if is_incremental() %}
{#- INVARIANT: the boundary is per record; a per-source maximum drops every record a new stream or a backfill brings with an older version. -#}
LEFT JOIN (
    SELECT unique_key, max(_version) AS stored_version
    FROM {{ this }}
    GROUP BY unique_key
) AS stored
    ON stored.unique_key = {{ cte }}.unique_key
{% endif %}
{#- INVARIANT: the latest version is chosen over (unique_key, _version) alone; sorting the wide rows exhausts server memory. -#}
WHERE ({{ cte }}.unique_key, {{ cte }}._version) IN (
    SELECT unique_key, max(_version)
    FROM {{ cte }}
    GROUP BY unique_key
)
{% if is_incremental() %}
  AND {{ cte }}._version > coalesce(stored.stored_version, 0)
{% endif %}
LIMIT 1 BY {{ cte }}.unique_key
{% endmacro %}


{% macro hubspot_pipeline_polls() %}
{#-
  The latest row of every pipeline, with the time of the poll it belongs to.
  The connector re-reads every pipeline of an object type in one call each
  sync, so a pipeline whose latest row predates its object type's latest
  poll is gone from HubSpot.
-#}
    {% for tbl in ['deal_pipelines', 'ticket_pipelines'] %}
    SELECT
        tenant_id,
        source_id,
        objectType                                      AS object_type,
        id                                              AS pipeline_id,
        label,
        displayOrder,
        stages,
        archived,
        createdAt,
        updatedAt,
        data_source,
        _airbyte_extracted_at,
        max(_airbyte_extracted_at) OVER (PARTITION BY tenant_id, source_id, objectType) AS polled_at
    FROM {{ source('bronze_hubspot', tbl) }} FINAL
    {% if not loop.last %}UNION ALL{% endif %}
    {% endfor %}
{% endmacro %}


{% macro hubspot_absent_from_poll(extracted_at, polled_at) -%}
  {#- INVARIANT: one sync reads a whole object type's pipelines within seconds;
      a row older than its poll by more than this window was not in it. -#}
  {{ extracted_at }} < {{ polled_at }} - INTERVAL 10 MINUTE
{%- endmacro %}


{% macro hubspot_known_time(column) -%}
  {#- INVARIANT: matches MISSING_CURSOR_TIMESTAMP, which the connector stamps on a record HubSpot sent without a cursor. -#}
  nullIf({{ column }}, toDateTime64('1970-01-01 00:00:00', 3, 'UTC'))
{%- endmacro %}
