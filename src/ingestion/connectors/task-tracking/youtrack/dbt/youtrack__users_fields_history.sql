{{ config(materialized='view', schema='staging', tags=['youtrack', 'staging']) }}

{{ fields_history(snapshot_ref=ref('youtrack__users_snapshot'), entity_id_col='id', fields=['email', 'fullName', 'login', 'banned', 'isAnonymized']) }}
