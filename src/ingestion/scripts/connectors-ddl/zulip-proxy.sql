CREATE DATABASE IF NOT EXISTS `bronze_zulip_proxy`;

CREATE TABLE IF NOT EXISTS bronze_zulip_proxy.messages
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `uniq` Nullable(String),
    `sender_id` Nullable(Decimal(38, 9)),
    `count` Nullable(Decimal(38, 9)),
    `created_at` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_zulip_proxy.users
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(Decimal(38, 9)),
    `uuid` Nullable(String),
    `email` Nullable(String),
    `full_name` Nullable(String),
    `role` Nullable(Decimal(38, 9)),
    `is_active` Nullable(Bool),
    `recipient_id` Nullable(Decimal(38, 9)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

