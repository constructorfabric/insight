-- depends_on: {{ ref('jira__bronze_promoted') }}
{# `staging` tag (issue #1893): the prod jira pipeline materializes staging
   feeders only in its staging step (`tag:staging,tag:jira`) and builds silver
   with `tag:silver,tag:jira+`. Tagged only `jira`, this model matched neither
   pass, so on a fresh install `staging.jira__users_snapshot` was never built and
   the silver model `jira__users_fields_history` — which ref()s it — failed with
   `code: 60 Unknown table expression identifier 'staging.jira__users_snapshot'`.
   `schema='staging'` is the target DATABASE, not a dbt tag, so it does not
   participate in tag selection. #}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['jira', 'staging']
) }}

{{ snapshot(
    source_ref=source('bronze_jira', 'jira_user'),
    unique_key_col='unique_key',
    check_cols=[
        'display_name', 'email', 'active', 'account_type'
    ]
) }}
