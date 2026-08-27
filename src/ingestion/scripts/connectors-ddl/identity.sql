CREATE DATABASE IF NOT EXISTS `identity`;

CREATE TABLE IF NOT EXISTS identity.identity_inputs
(
    `unique_key` String,
    `insight_tenant_id` UUID,
    `insight_source_id` UUID,
    `insight_source_type` String,
    `source_account_id` Nullable(String),
    `value_type` String,
    `value` Nullable(String),
    `value_field_name` String,
    `operation_type` String,
    `_synced_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS identity.identity_persons
(
    `id` UInt64,
    `value_type` String,
    `insight_source_type` String,
    `insight_source_id` UUID,
    `insight_tenant_id` UUID,
    `value_id` Nullable(String),
    `value_full_text` Nullable(String),
    `value` Nullable(String),
    `value_effective` Nullable(String),
    `person_id` UUID,
    `author_person_id` UUID,
    `reason` Nullable(String),
    `created_at` DateTime64(6, 'UTC'),
    `_synced_at` DateTime64(3, 'UTC')
)
ENGINE = MergeTree
ORDER BY id
SETTINGS index_granularity = 8192
;

