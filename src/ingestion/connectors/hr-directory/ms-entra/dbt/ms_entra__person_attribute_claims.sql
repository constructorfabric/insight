-- depends_on: {{ ref('ms_entra__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    tags=['ms-entra', 'silver:class_person_attribute_claims']
) }}

{{ attribute_claims(
    snapshot_ref=ref('ms_entra__users_snapshot'),
    entity_id_col='id',
    source_type='ms-entra',
    fields=['department', 'jobTitle', 'userType', 'accountEnabled']
) }}
