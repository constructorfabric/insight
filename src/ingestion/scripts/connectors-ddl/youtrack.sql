CREATE DATABASE IF NOT EXISTS `bronze_youtrack`;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_activities
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `_type` Nullable(String),
    `timestamp` Nullable(String),
    `targetMember` Nullable(String),
    `author_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `category_json` Nullable(String),
    `field_json` Nullable(String),
    `added_json` Nullable(String),
    `removed_json` Nullable(String),
    `target_json` Nullable(String),
    `activity_json` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_agiles
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `name` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `agile_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `text` Nullable(String),
    `created` Nullable(Decimal(38, 9)),
    `updated` Nullable(String),
    `deleted` Nullable(Bool),
    `issue_id` Nullable(String),
    `issue_id_readable` Nullable(String),
    `issue_updated` Nullable(String),
    `author_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `comment_json` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_custom_fields
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `name` Nullable(String),
    `field_type_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `metadata_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_field_values
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `project_id` Nullable(String),
    `field_id` Nullable(String),
    `field_type_id` Nullable(String),
    `bundle_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `bundle_json` Nullable(String),
    `values_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issue_census
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `idReadable` Nullable(String),
    `created` Nullable(Decimal(38, 9)),
    `updated` Nullable(Decimal(38, 9)),
    `resolved` Nullable(Decimal(38, 9)),
    `project_id` Nullable(String),
    `observed_at` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issue_keys
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `idReadable` Nullable(String),
    `updated` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issue_links
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `direction` Nullable(String),
    `issue_id` Nullable(String),
    `issue_id_readable` Nullable(String),
    `issue_updated` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `linked_issues_json` Nullable(String),
    `link_json` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issue_sprints
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `sprint_id` Nullable(String),
    `issue_id` Nullable(String),
    `issue_id_readable` Nullable(String),
    `issue_updated` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `sprint_json` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_issues
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `_type` Nullable(String),
    `idReadable` Nullable(String),
    `summary` Nullable(String),
    `description` Nullable(String),
    `created` Nullable(Decimal(38, 9)),
    `updated` Nullable(String),
    `resolved` Nullable(Decimal(38, 9)),
    `numberInProject` Nullable(Decimal(38, 9)),
    `isDraft` Nullable(Bool),
    `commentsCount` Nullable(Decimal(38, 9)),
    `votes` Nullable(Decimal(38, 9)),
    `wikifiedDescription` Nullable(String),
    `project_id` Nullable(String),
    `reporter_id` Nullable(String),
    `updater_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `custom_fields_json` Nullable(String),
    `issue_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_project_fields
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `_type` Nullable(String),
    `project_id` Nullable(String),
    `field_type_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `metadata_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_projects
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `_type` Nullable(String),
    `name` Nullable(String),
    `shortName` Nullable(String),
    `description` Nullable(String),
    `archived` Nullable(Bool),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `project_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_sprints
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `name` Nullable(String),
    `start` Nullable(Decimal(38, 9)),
    `finish` Nullable(Decimal(38, 9)),
    `archived` Nullable(Bool),
    `agile_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `sprint_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_users
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `login` Nullable(String),
    `fullName` Nullable(String),
    `email` Nullable(String),
    `banned` Nullable(Bool),
    `isAnonymized` Nullable(Bool),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `user_json` Nullable(String),
    `observed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_youtrack.youtrack_work_items
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `date` Nullable(Decimal(38, 9)),
    `created` Nullable(Decimal(38, 9)),
    `updated` Nullable(String),
    `issue_id` Nullable(String),
    `author_id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `work_item_json` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

