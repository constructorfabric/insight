{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['slack']
) }}

{{ snapshot(
    source_ref=ref('slack__users_latest'),
    unique_key_col='unique_key',
    check_cols=[
        'email',
        'is_guest',
        'is_billable_seat'
    ]
) }}
