-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['jira']
) }}

{{ snapshot(
    source_ref=source('bronze_jira', 'jira_user'),
    unique_key_col='unique_key',
    check_cols=[
        'display_name', 'email', 'active', 'account_type'
    ]
) }}
