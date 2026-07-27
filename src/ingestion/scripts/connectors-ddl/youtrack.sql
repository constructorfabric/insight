CREATE DATABASE IF NOT EXISTS `bronze_youtrack`;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_agiles
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `agile_id` Nullable(String),
    `name` Nullable(String),
    `owner_id` Nullable(String),
    `projects` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `comment_id` Nullable(String),
    `author_id` Nullable(String),
    `author_login` Nullable(String),
    `author_email` Nullable(String),
    `text` Nullable(String),
    `text_preview` Nullable(String),
    `created` Nullable(Int64),
    `updated` Nullable(Int64),
    `deleted` Nullable(Bool),
    `collected_at` Nullable(String),
    `youtrack_id` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issue
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `youtrack_id` Nullable(String),
    `id_readable` Nullable(String),
    `project_short_name` Nullable(String),
    `summary` Nullable(String),
    `description` Nullable(String),
    `reporter_id` Nullable(String),
    `reporter_login` Nullable(String),
    `reporter_email` Nullable(String),
    `created` Nullable(Int64),
    `updated` Nullable(Int64),
    `resolved` Nullable(Int64),
    `tags_json` Nullable(String),
    `attachments_json` Nullable(String),
    `links_json` Nullable(String),
    `custom_fields_json` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issue_history
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `activity_id` Nullable(String),
    `activity_type` Nullable(String),
    `category_id` Nullable(String),
    `category_name` Nullable(String),
    `author_id` Nullable(String),
    `author_login` Nullable(String),
    `field_id` Nullable(String),
    `field_custom_id` Nullable(String),
    `field_value_type` Nullable(String),
    `field_is_multi_value` Nullable(Bool),
    `field_presentation` Nullable(String),
    `target_member` Nullable(String),
    `added_json` Nullable(String),
    `removed_json` Nullable(String),
    `timestamp` Nullable(Int64),
    `collected_at` Nullable(String),
    `youtrack_id` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issue_link_types
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `link_type_id` Nullable(String),
    `name` Nullable(String),
    `source_to_target` Nullable(String),
    `target_to_source` Nullable(String),
    `directed` Nullable(Bool),
    `aggregation` Nullable(Bool),
    `read_only` Nullable(Bool),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_project_custom_fields
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_id` Nullable(String),
    `project_custom_field_id` Nullable(String),
    `field_id` Nullable(String),
    `field_name` Nullable(String),
    `field_localized_name` Nullable(String),
    `field_type_id` Nullable(String),
    `value_type` Nullable(String),
    `is_multi_value` Nullable(Bool),
    `can_be_empty` Nullable(Bool),
    `empty_field_text` Nullable(String),
    `ordinal` Nullable(Int64),
    `is_public` Nullable(Bool),
    `bundle_id` Nullable(String),
    `bundle_values_json` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_projects
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_id` Nullable(String),
    `short_name` Nullable(String),
    `name` Nullable(String),
    `description` Nullable(String),
    `archived` Nullable(Bool),
    `leader_id` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_sprints
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `sprint_id` Nullable(String),
    `agile_id` Nullable(String),
    `sprint_name` Nullable(String),
    `start_date` Nullable(Int64),
    `finish_date` Nullable(Int64),
    `archived` Nullable(Bool),
    `goal` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_user
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `user_id` Nullable(String),
    `login` Nullable(String),
    `full_name` Nullable(String),
    `email` Nullable(String),
    `banned` Nullable(Bool),
    `guest` Nullable(Bool),
    `online` Nullable(Bool),
    `avatar_url` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_worklogs
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `worklog_id` Nullable(String),
    `author_id` Nullable(String),
    `author_login` Nullable(String),
    `author_email` Nullable(String),
    `creator_id` Nullable(String),
    `date` Nullable(Int64),
    `duration_minutes` Nullable(Int64),
    `text` Nullable(String),
    `work_type_id` Nullable(String),
    `work_type_name` Nullable(String),
    `collected_at` Nullable(String),
    `youtrack_id` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

