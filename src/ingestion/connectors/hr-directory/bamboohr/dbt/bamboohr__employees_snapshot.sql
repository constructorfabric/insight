-- depends_on: {{ ref('bamboohr__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['bamboohr']
) }}

{{ snapshot(
    source_ref=source('bamboohr', 'employees'),
    unique_key_col='unique_key',
    check_cols=[],
    check_raw_data_all=true,
    raw_data_exclude_keys=['lastChanged']
) }}
