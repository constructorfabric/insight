{{ config(materialized='incremental', full_refresh=false, incremental_strategy='append', schema='staging',
          engine='ReplacingMergeTree(_version)', order_by=['unique_key'],
          settings={'allow_nullable_key': 1}, tags=['youtrack', 'staging']) }}

-- Columns by name: the append inserts by position, and the destination owns
-- the Bronze column order (it rebuilds the table on a key change, #2877).
SELECT _airbyte_raw_id, _airbyte_extracted_at, _airbyte_meta, _airbyte_generation_id,
    id, date, created, updated, issue_id, author_id, tenant_id, source_id, work_item_json,
    concat(coalesce(unique_key, ''), '-', toString(_airbyte_extracted_at)) AS unique_key,
    toUInt64(toUnixTimestamp64Milli(_airbyte_extracted_at)) AS _version
FROM {{ source('bronze_youtrack', 'youtrack_work_items') }} FINAL
