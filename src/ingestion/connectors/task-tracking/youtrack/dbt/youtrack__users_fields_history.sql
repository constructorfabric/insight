-- depends_on: {{ ref('youtrack__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['youtrack', 'silver']
) }}

-- Field-level change log of the YouTrack user profile, derived from the snapshot.
-- Input to youtrack__identity_inputs via identity_inputs_from_history.
-- entity_id = YouTrack internal user id. Mirrors jira__users_fields_history.

{{ fields_history(
    snapshot_ref=ref('youtrack__users_snapshot'),
    entity_id_col='user_id',
    fields=[
        'full_name', 'email', 'login', 'banned'
    ]
) }}
