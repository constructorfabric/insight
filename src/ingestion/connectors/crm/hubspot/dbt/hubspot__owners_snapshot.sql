{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['hubspot']
) }}

{{ snapshot(
    source_ref=ref('hubspot__owners_latest'),
    unique_key_col='unique_key',
    check_cols=['email', 'display_name', 'is_archived']
) }}
