-- depends_on: {{ ref('jira__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['jira', 'staging']
) }}

{{ snapshot(
    source_ref=ref('jira__project_visibility_state'),
    unique_key_col='unique_key',
    check_cols=[
        'is_visible', 'project_status'
    ]
) }}
