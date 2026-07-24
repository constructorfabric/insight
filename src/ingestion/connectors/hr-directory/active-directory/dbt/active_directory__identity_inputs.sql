{{ config(
    materialized='incremental',
    incremental_strategy='append',
    schema='staging',
    tags=['active-directory', 'silver', 'silver:identity_inputs']
) }}

{# Emit identity signals from the on-prem Active Directory user directory.

   `userPrincipalName` and `mail` both yield value_type='email' so the Identity
   Manager can match a person across services regardless of which email form a
   downstream connector recorded.

   `sAMAccountName` carries the legacy AD/SAM login — the SAME value that
   ms-entra surfaces as `onPremisesSamAccountName`. Emitting it as value_type
   'sam_account' lets the Identity Manager reconcile this on-prem connector with
   the cloud ms-entra connector AND with self-hosted services (Bitbucket Server,
   GitLab self-hosted) where SAM is often the username.

   `proxyAddresses` is array-valued in Bronze; the macro operates on scalar
   history rows, so alternate addresses are added in a follow-up
   (REC-IR-07: array-valued identity inputs) — same limitation as ms-entra.
#}

{{ identity_inputs_from_history(
    fields_history_ref=ref('active_directory__users_fields_history'),
    source_type='active-directory',
    identity_fields=[
        {'field': 'mail',              'value_type': 'email',        'value_field_name': 'bronze_active_directory.users.mail'},
        {'field': 'userPrincipalName', 'value_type': 'email',        'value_field_name': 'bronze_active_directory.users.userPrincipalName'},
        {'field': 'employeeId',        'value_type': 'employee_id',  'value_field_name': 'bronze_active_directory.users.employeeId'},
        {'field': 'displayName',       'value_type': 'display_name', 'value_field_name': 'bronze_active_directory.users.displayName'},
        {'field': 'sAMAccountName',    'value_type': 'sam_account',  'value_field_name': 'bronze_active_directory.users.sAMAccountName'},
        {'field': 'status',            'value_type': 'status',       'value_field_name': 'bronze_active_directory.users.status'},
    ],
    deactivation_condition="field_name = 'status' AND new_value = 'Terminated'"
) }}
