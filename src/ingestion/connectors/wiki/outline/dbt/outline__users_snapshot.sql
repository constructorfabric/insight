-- depends_on: {{ ref('outline__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['outline']
) }}

{{ snapshot(
    source_ref=source('bronze_outline', 'wiki_users'),
    unique_key_col='unique_key',
    check_cols=[
        'name', 'email', 'role', 'is_suspended'
    ]
) }}
