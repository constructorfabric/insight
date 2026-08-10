-- depends_on: {{ ref('workday__bronze_promoted') }}
{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    engine='ReplacingMergeTree(_version)',
    order_by=['unique_key'],
    tags=['workday', 'silver:class_person_attribute_claims']
) }}

{{ attribute_claims(
    snapshot_ref=ref('workday__workers_snapshot'),
    entity_id_col='Employee_ID',
    source_type='workday',
    fields=[
        'Business_Title', 'Job_Profile', 'Worker_Type', 'Worker_Status',
        'Supervisory_Organization',
        'Location', 'Country', 'City'
    ]
) }}
