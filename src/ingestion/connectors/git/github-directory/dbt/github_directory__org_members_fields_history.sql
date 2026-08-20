-- depends_on: {{ ref('github_directory__bronze_promoted') }}
{{ config(
    materialized='table',
    schema='staging',
    tags=['github-directory', 'silver']
) }}

-- Field-level change log of the GitHub member profile, derived from the
-- snapshot. Input to github_directory__identity_inputs.
--
-- entity_id is the member's immutable numeric GitHub id (`databaseId`),
-- stringified: it becomes the `value_type='id'` binding that the
-- authenticator matches against the IdP's external-id claim, so the broker
-- must carry the same numeric id as a string. A login would silently re-key
-- the account on rename or reuse; the id cannot. `login` stays a tracked
-- field so a rename updates the `username` observation.
-- Mirrors youtrack__users_fields_history.

{{ fields_history(
    snapshot_ref=ref('github_directory__org_members_snapshot'),
    entity_id_col='toString(member_id)',
    fields=[
        'login', 'name', 'email'
    ]
) }}
