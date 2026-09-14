{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['claude-team']
) }}

{{ snapshot(
    source_ref=ref('claude_team__members_latest'),
    unique_key_col='unique_key',
    check_cols=[
        'email_address', 'full_name'
    ]
) }}
