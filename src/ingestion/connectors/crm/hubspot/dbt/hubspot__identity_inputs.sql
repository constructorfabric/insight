{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['hubspot', 'silver', 'silver:identity_inputs']
) }}

-- INVARIANT: no deactivation_condition. An archived owner is a deactivated
-- HubSpot user whose deals and engagements still name them; deleting the
-- binding would orphan that history from the person.

{{ identity_inputs_from_history(
    fields_history_ref=ref('hubspot__owners_fields_history'),
    source_type='hubspot',
    identity_fields=[
        {'field': 'email',        'value_type': 'email',        'value_field_name': 'bronze_hubspot.owners.email'},
        {'field': 'display_name', 'value_type': 'display_name', 'value_field_name': 'bronze_hubspot.owners.firstName+lastName'},
    ]
) }}
