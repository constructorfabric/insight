CREATE DATABASE IF NOT EXISTS `bronze_confluence`;

CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_footer_comment_replies
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
    `page_id` Nullable(String),
    `parent_comment_id` Nullable(String),
    `author_id` Nullable(String),
    `created_at` Nullable(String),
    `body_storage` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_footer_comments
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
    `page_id` Nullable(String),
    `author_id` Nullable(String),
    `created_at` Nullable(String),
    `body_storage` Nullable(String),
    `resolution_status` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_inline_comment_replies
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
    `page_id` Nullable(String),
    `parent_comment_id` Nullable(String),
    `author_id` Nullable(String),
    `created_at` Nullable(String),
    `body_storage` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_inline_comments
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
    `page_id` Nullable(String),
    `author_id` Nullable(String),
    `created_at` Nullable(String),
    `body_storage` Nullable(String),
    `resolution_status` Nullable(String),
    `inline_marker_ref` Nullable(String),
    `inline_original_selection` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_page_versions
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `page_id` Nullable(String),
    `version_number` Nullable(Int64),
    `author_id` Nullable(String),
    `created_at` Nullable(String),
    `message` Nullable(String),
    `minor_edit` Nullable(Bool),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_pages
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `page_id` Nullable(String),
    `space_id` Nullable(String),
    `title` Nullable(String),
    `status` Nullable(String),
    `author_id` Nullable(String),
    `last_editor_id` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `version_number` Nullable(Int64),
    `parent_page_id` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_confluence.wiki_spaces
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `space_id` Nullable(String),
    `name` Nullable(String),
    `description` Nullable(String),
    `space_type` Nullable(String),
    `status` Nullable(String),
    `url` Nullable(String),
    `created_at` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

