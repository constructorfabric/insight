CREATE DATABASE IF NOT EXISTS `bronze_github_directory`;

CREATE TABLE IF NOT EXISTS bronze_github_directory.org_members
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String),
    `org` Nullable(String),
    `login` Nullable(String),
    `member_id` Nullable(Int64),
    `name` Nullable(String),
    `email` Nullable(String),
    `company` Nullable(String),
    `role` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

