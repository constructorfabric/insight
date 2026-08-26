-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['jira', 'staging']
) }}

{{ snapshot(
    source_ref=ref('jira__comment_state'),
    unique_key_col='unique_key',
    check_cols=[
        'edited_at', 'is_deleted'
    ]
) }}
