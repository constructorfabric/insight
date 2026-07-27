CREATE DATABASE IF NOT EXISTS `bronze_figma`;

CREATE TABLE IF NOT EXISTS bronze_figma.design_file_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `comment_id` Nullable(String),
    `file_key` Nullable(String),
    `parent_comment_id` Nullable(String),
    `author_id` Nullable(String),
    `author_handle` Nullable(String),
    `created_at` Nullable(String),
    `resolved_at` Nullable(String),
    `message` Nullable(String),
    `order_id` Nullable(String),
    `reaction_count` Nullable(Int64),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_figma.design_file_meta
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `file_key` Nullable(String),
    `folder_name` Nullable(String),
    `creator_id` Nullable(String),
    `creator_handle` Nullable(String),
    `last_touched_by_id` Nullable(String),
    `last_touched_by_handle` Nullable(String),
    `last_touched_at` Nullable(String),
    `editor_type` Nullable(String),
    `link_access` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_figma.design_file_versions
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `version_id` Nullable(String),
    `file_key` Nullable(String),
    `created_at` Nullable(String),
    `label` Nullable(String),
    `description` Nullable(String),
    `author_id` Nullable(String),
    `author_handle` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_figma.design_files
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `file_key` Nullable(String),
    `file_name` Nullable(String),
    `project_id` Nullable(String),
    `project_name` Nullable(String),
    `team_id` Nullable(String),
    `last_modified` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_figma.design_projects
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `project_id` Nullable(String),
    `project_name` Nullable(String),
    `team_id` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

