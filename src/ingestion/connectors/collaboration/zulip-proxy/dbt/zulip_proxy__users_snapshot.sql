-- depends_on: {{ ref('zulip_proxy__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['zulip-proxy']
) }}

{{ snapshot(
    source_ref=source('bronze_zulip_proxy', 'users'),
    unique_key_col='unique_key',
    check_cols=[
        'full_name', 'email', 'is_active', 'role', 'recipient_id', 'uuid'
    ]
) }}
