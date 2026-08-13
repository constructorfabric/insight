-- depends_on: {{ ref('bamboohr__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['bamboohr', 'silver']
) }}

{{ fields_history_raw(
    snapshot_ref=ref('bamboohr__employees_snapshot'),
    entity_id_col='id',
    exclude_keys=['lastChanged']
) }}
