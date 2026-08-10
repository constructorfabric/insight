-- depends_on: {{ ref('bamboohr__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    tags=['bamboohr', 'silver:class_person_attribute_claims']
) }}

{{ attribute_claims(
    snapshot_ref=ref('bamboohr__employees_snapshot'),
    entity_id_col='id',
    source_type='bamboohr',
    fields=[
        'jobTitle', 'department', 'division',
        'status', 'employmentHistoryStatus',
        'location', 'country', 'city'
    ]
) }}
