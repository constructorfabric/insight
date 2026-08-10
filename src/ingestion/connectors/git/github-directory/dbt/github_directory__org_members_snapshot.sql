-- depends_on: {{ ref('github_directory__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['github_directory']
) }}

-- SCD2 snapshot of the GitHub organization roster — appends a new version only
-- when a tracked profile field changes. Feeds
-- github_directory__org_members_fields_history →
-- github_directory__identity_inputs. Mirrors youtrack__users_snapshot.

{{ snapshot(
    source_ref=source('bronze_github_directory', 'org_members'),
    unique_key_col='unique_key',
    check_cols=[
        'login', 'name', 'email'
    ]
) }}
