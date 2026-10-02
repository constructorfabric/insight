{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging', 'silver:identity_inputs']) }}

{{ identity_inputs_from_history(
    fields_history_ref=ref('youtrack__users_fields_history'), source_type='youtrack',
    identity_fields=[
        {'field': 'email', 'value_type': 'email', 'value_field_name': 'bronze_youtrack.youtrack_users.email'},
        {'field': 'fullName', 'value_type': 'display_name', 'value_field_name': 'bronze_youtrack.youtrack_users.fullName'}
    ], deactivation_condition="field_name IN ('banned', 'isAnonymized') AND lower(new_value) IN ('true', '1')"
) }}
