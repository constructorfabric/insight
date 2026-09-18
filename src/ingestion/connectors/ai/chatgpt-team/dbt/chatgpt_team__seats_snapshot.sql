{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['chatgpt-team']
) }}

{{ snapshot(
    source_ref=ref('chatgpt_team__seats_latest'),
    unique_key_col='unique_key',
    check_cols=[
        'email', 'name', 'deactivated_time'
    ]
) }}
