-- depends_on: {{ ref('active_directory__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    tags=['active-directory', 'silver:class_person_attribute_claims']
) }}

{{ attribute_claims(
    snapshot_ref=ref('active_directory__users_snapshot'),
    entity_id_col='id',
    source_type='active-directory',
    fields=['department', 'jobTitle', 'status', 'accountEnabled']
) }}
