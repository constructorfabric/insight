{{ config(materialized='incremental', full_refresh=false, schema='staging', tags=['youtrack', 'staging'], engine='ReplacingMergeTree', order_by=['unique_key', '_tracked_at'], settings={'allow_nullable_key': 1}, incremental_strategy='append') }}

-- Snapshot versions share the entity key; capture time must distinguish their replacement identities.
{{ snapshot(source_ref=source('bronze_youtrack', 'youtrack_users'), unique_key_col='unique_key', check_cols=['email', 'fullName', 'login', 'banned', 'isAnonymized']) }}
