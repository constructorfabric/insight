CREATE DATABASE IF NOT EXISTS `bronze_slack`;

CREATE TABLE IF NOT EXISTS bronze_slack.users_details
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `date` Nullable(String),
    `team_id` Nullable(String),
    `user_id` Nullable(String),
    `email_address` Nullable(String),
    `is_guest` Nullable(Bool),
    `is_billable_seat` Nullable(Bool),
    `is_active` Nullable(Bool),
    `is_active_ios` Nullable(Bool),
    `is_active_android` Nullable(Bool),
    `is_active_desktop` Nullable(Bool),
    `is_active_apps` Nullable(Bool),
    `is_active_workflows` Nullable(Bool),
    `is_active_slack_connect` Nullable(Bool),
    `reactions_added_count` Nullable(Int64),
    `messages_posted_count` Nullable(Int64),
    `channel_messages_posted_count` Nullable(Int64),
    `files_added_count` Nullable(Int64),
    `total_calls_count` Nullable(Int64),
    `slack_calls_count` Nullable(Int64),
    `slack_huddles_count` Nullable(Int64),
    `search_count` Nullable(Int64),
    `date_claimed` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

