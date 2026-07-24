CREATE DATABASE IF NOT EXISTS `identity`;

CREATE TABLE IF NOT EXISTS identity.aliases
(
    `id` UUID DEFAULT generateUUIDv7(),
    `insight_tenant_id` UUID,
    `person_id` UUID,
    `value_type` LowCardinality(String),
    `value` String,
    `value_field_name` String DEFAULT '',
    `insight_source_id` UUID DEFAULT toUUID('00000000-0000-0000-0000-000000000000'),
    `insight_source_type` LowCardinality(String) DEFAULT '',
    `source_account_id` String DEFAULT '',
    `confidence` Float32 DEFAULT 1.,
    `is_active` UInt8 DEFAULT 1,
    `effective_from` DateTime64(3, 'UTC') DEFAULT now64(3),
    `effective_to` DateTime64(3, 'UTC') DEFAULT toDateTime64('1970-01-01 00:00:00.000', 3, 'UTC'),
    `first_observed_at` DateTime64(3, 'UTC') DEFAULT now64(3),
    `last_observed_at` DateTime64(3, 'UTC') DEFAULT now64(3),
    `created_at` DateTime64(3, 'UTC') DEFAULT now64(3),
    `updated_at` DateTime64(3, 'UTC') DEFAULT now64(3),
    `is_deleted` UInt8 DEFAULT 0
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (insight_tenant_id, value_type, value, insight_source_id, id)
SETTINGS index_granularity = 8192
;

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

CREATE TABLE IF NOT EXISTS identity.seed_aliases_from_claude_admin
(
    `id` UUID,
    `insight_tenant_id` UUID,
    `person_id` UUID,
    `value_type` String,
    `value` Nullable(String),
    `value_field_name` String,
    `insight_source_id` UUID,
    `insight_source_type` String,
    `source_account_id` Nullable(String),
    `confidence` Float32,
    `is_active` UInt8,
    `effective_from` DateTime64(3),
    `effective_to` DateTime64(3, 'UTC'),
    `first_observed_at` DateTime64(3),
    `last_observed_at` DateTime64(3),
    `created_at` DateTime64(3),
    `updated_at` DateTime64(3),
    `is_deleted` UInt8,
    `_version` Int64
)
ENGINE = MergeTree
ORDER BY id
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS identity.seed_aliases_from_cursor
(
    `id` UUID,
    `insight_tenant_id` String,
    `person_id` UUID,
    `value_type` String,
    `value` Nullable(String),
    `value_field_name` String,
    `insight_source_id` UUID,
    `insight_source_type` String,
    `source_account_id` Nullable(String),
    `confidence` Float32,
    `is_active` UInt8,
    `effective_from` DateTime64(3),
    `effective_to` DateTime64(3, 'UTC'),
    `first_observed_at` DateTime64(3),
    `last_observed_at` DateTime64(3),
    `created_at` DateTime64(3),
    `updated_at` DateTime64(3),
    `is_deleted` UInt8,
    `_version` Int64
)
ENGINE = MergeTree
ORDER BY id
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

