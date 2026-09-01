{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['zulip-proxy', 'silver', 'silver:identity_inputs']
) }}

{{ identity_inputs_from_history(
    fields_history_ref=ref('zulip_proxy__users_fields_history'),
    source_type='zulip_proxy',
    identity_fields=[
        {'field': 'email',     'value_type': 'email',        'value_field_name': 'bronze_zulip_proxy.users.email'},
        {'field': 'full_name', 'value_type': 'display_name', 'value_field_name': 'bronze_zulip_proxy.users.full_name'},
    ],
    deactivation_condition="field_name = 'is_active' AND lower(new_value) = 'false'"
) }}
