{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['outline', 'silver', 'silver:identity_inputs']
) }}

{{ identity_inputs_from_history(
    fields_history_ref=ref('outline__users_fields_history'),
    source_type='outline',
    identity_fields=[
        {'field': 'email', 'value_type': 'email',        'value_field_name': 'bronze_outline.wiki_users.email'},
        {'field': 'name',  'value_type': 'display_name', 'value_field_name': 'bronze_outline.wiki_users.name'},
    ],
    deactivation_condition="field_name = 'is_suspended' AND lower(new_value) = 'true'"
) }}
