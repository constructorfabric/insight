CREATE DATABASE IF NOT EXISTS `bronze_compass`;

CREATE TABLE IF NOT EXISTS bronze_compass.component_scorecard_scores
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
    `scorecard_id` Nullable(String),
    `component_id` Nullable(String),
    `total_score` Nullable(Int64),
    `max_total_score` Nullable(Int64),
    `status` Nullable(String),
    `criteria_scores` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_compass.components
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
    `component_id` Nullable(String),
    `name` Nullable(String),
    `slug` Nullable(String),
    `component_type` Nullable(String),
    `description` Nullable(String),
    `component_url` Nullable(String),
    `owner_team_id` Nullable(String),
    `labels` Nullable(String),
    `links` Nullable(String),
    `event_sources` Nullable(String),
    `relationships` Nullable(String),
    `relationships_truncated` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_compass.deployment_events
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
    `component_id` Nullable(String),
    `event_type` Nullable(String),
    `display_name` Nullable(String),
    `description` Nullable(String),
    `event_url` Nullable(String),
    `last_updated` Nullable(String),
    `update_sequence_number` Nullable(Int64),
    `state` Nullable(String),
    `environment_category` Nullable(String),
    `environment_display_name` Nullable(String),
    `environment_id` Nullable(String),
    `started_at` Nullable(String),
    `completed_at` Nullable(String),
    `sequence_number` Nullable(Int64),
    `pipeline` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_compass.scorecards
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
    `scorecard_id` Nullable(String),
    `name` Nullable(String),
    `description` Nullable(String),
    `state` Nullable(String),
    `importance` Nullable(String),
    `created_at` Nullable(String),
    `last_user_modification_at` Nullable(String),
    `criterias` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_compass.team_members
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
    `team_id` Nullable(String),
    `member_id` Nullable(String),
    `member_name` Nullable(String),
    `account_status` Nullable(String),
    `role` Nullable(String),
    `membership_state` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_compass.teams
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
    `team_id` Nullable(String),
    `display_name` Nullable(String),
    `description` Nullable(String),
    `state` Nullable(String),
    `organization_id` Nullable(String),
    `member_count` Nullable(Int64),
    `is_verified` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

