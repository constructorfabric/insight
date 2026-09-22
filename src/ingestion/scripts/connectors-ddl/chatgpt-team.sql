CREATE DATABASE IF NOT EXISTS `bronze_chatgpt_team`;

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_account_settings
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
    `seat_type_credit_limits` Nullable(String),
    `default_seat_type` Nullable(String),
    `auto_approve_seat_upgrades` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

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

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_codex_sessions_daily
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
    `user_id` Nullable(String),
    `n_new_sessions_total` Nullable(Decimal(38, 9)),
    `n_user_messages_total` Nullable(Decimal(38, 9)),
    `n_tasks_web` Nullable(Decimal(38, 9)),
    `n_code_reviews_web` Nullable(Decimal(38, 9)),
    `credit_total` Nullable(Decimal(38, 9)),
    `on_demand_credits` Nullable(Decimal(38, 9)),
    `uncached_text_input_tokens` Nullable(Decimal(38, 9)),
    `cached_text_input_tokens` Nullable(Decimal(38, 9)),
    `text_output_tokens` Nullable(Decimal(38, 9)),
    `text_total_tokens` Nullable(Decimal(38, 9)),
    `credit_cli` Nullable(Decimal(38, 9)),
    `credit_vscode` Nullable(Decimal(38, 9)),
    `credit_exec` Nullable(Decimal(38, 9)),
    `credit_sdk_ts` Nullable(Decimal(38, 9)),
    `credit_desktop` Nullable(Decimal(38, 9)),
    `credit_web` Nullable(Decimal(38, 9)),
    `credit_slack` Nullable(Decimal(38, 9)),
    `credit_github_code_review` Nullable(Decimal(38, 9)),
    `credit_github_turn` Nullable(Decimal(38, 9)),
    `n_new_sessions_cli` Nullable(Decimal(38, 9)),
    `n_new_sessions_vscode` Nullable(Decimal(38, 9)),
    `n_new_sessions_exec` Nullable(Decimal(38, 9)),
    `n_new_sessions_sdk_ts` Nullable(Decimal(38, 9)),
    `n_new_sessions_desktop` Nullable(Decimal(38, 9)),
    `n_new_sessions_work_desktop` Nullable(Decimal(38, 9)),
    `n_new_sessions_work_web` Nullable(Decimal(38, 9)),
    `n_new_sessions_work_mobile` Nullable(Decimal(38, 9)),
    `n_new_sessions_other` Nullable(Decimal(38, 9)),
    `n_user_messages_cli` Nullable(Decimal(38, 9)),
    `n_user_messages_vscode` Nullable(Decimal(38, 9)),
    `n_user_messages_exec` Nullable(Decimal(38, 9)),
    `n_user_messages_sdk_ts` Nullable(Decimal(38, 9)),
    `n_user_messages_desktop` Nullable(Decimal(38, 9)),
    `n_user_messages_work_desktop` Nullable(Decimal(38, 9)),
    `n_user_messages_work_web` Nullable(Decimal(38, 9)),
    `n_user_messages_work_mobile` Nullable(Decimal(38, 9)),
    `n_user_messages_other` Nullable(Decimal(38, 9))
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

CREATE TABLE IF NOT EXISTS bronze_chatgpt_team.chatgpt_team_codex_user_daily_org
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
    `total_users` Nullable(Decimal(38, 9))
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
    `added_at` Nullable(String),
    `account_user_id` Nullable(String),
    `verified_email` Nullable(String),
    `trial_state` Nullable(String),
    `is_trial` Nullable(Bool),
    `is_scim_managed` Nullable(Bool),
    `creation_source` Nullable(String),
    `deactivated_time` Nullable(String),
    `pending_seat_type` Nullable(String),
    `reclaimable_seat_type` Nullable(String),
    `credit_limits` Nullable(String)
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

