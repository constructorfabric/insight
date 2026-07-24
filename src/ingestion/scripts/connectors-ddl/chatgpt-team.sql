CREATE DATABASE IF NOT EXISTS `bronze_chatgpt_team`;

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_chat_activity
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
    `date` Nullable(String),
    `email` Nullable(String),
    `name` Nullable(String),
    `seat_type` Nullable(String),
    `messages` Nullable(Decimal(38, 9)),
    `gpt_messages` Nullable(Decimal(38, 9)),
    `tool_messages` Nullable(Decimal(38, 9)),
    `connector_messages` Nullable(Decimal(38, 9)),
    `project_messages` Nullable(Decimal(38, 9)),
    `credits_used` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_codex_user_daily
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
    `date` Nullable(String),
    `email` Nullable(String),
    `user_id` Nullable(String),
    `name` Nullable(String),
    `credits` Nullable(Decimal(38, 9)),
    `n_threads` Nullable(Decimal(38, 9)),
    `n_turns` Nullable(Decimal(38, 9)),
    `current_streak` Nullable(Decimal(38, 9)),
    `text_tokens` Nullable(Decimal(38, 9)),
    `lines_added` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_seats
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
    `user_id` Nullable(String),
    `email` Nullable(String),
    `name` Nullable(String),
    `role` Nullable(String),
    `seat_type` Nullable(String),
    `added_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_subscription_balance
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
    `snapshot_date` Nullable(String),
    `current_balance` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_subscription_usage
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
    `snapshot_date` Nullable(String),
    `model` Nullable(String),
    `amount` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

