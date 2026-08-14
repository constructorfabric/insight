-- depends_on: {{ ref('bamboohr__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    query_settings={
        'max_bytes_before_external_group_by': 268435456,
        'max_bytes_ratio_before_external_group_by': 0,
        'max_block_size': 8192,
        'max_threads': 4,
    },
    tags=['bamboohr', 'silver']
) }}

{#-
  fields_history_raw is shaped so its state is GROUP BY state, which spills to
  disk past external_group_by; small blocks and few threads bound the residual
  the pipeline holds in flight. No max_memory_usage: a self-imposed cap here
  turns a build the server could still afford into a hard failure.
-#}

{{ fields_history_raw(
    snapshot_ref=ref('bamboohr__employees_snapshot'),
    entity_id_col='id',
    exclude_keys=['lastChanged']
) }}
