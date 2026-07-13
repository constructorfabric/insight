-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['youtrack']
) }}

-- SCD2 snapshot of the YouTrack user directory — appends a new version only when
-- a tracked profile field changes. Feeds youtrack__users_fields_history →
-- youtrack__identity_inputs. Mirrors jira__users_snapshot.

{{ snapshot(
    source_ref=source('bronze_youtrack', 'youtrack_user'),
    unique_key_col='unique_key',
    check_cols=[
        'full_name', 'email', 'login', 'banned'
    ]
) }}
