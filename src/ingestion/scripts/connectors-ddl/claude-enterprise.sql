CREATE DATABASE IF NOT EXISTS `bronze_claude_enterprise`;

CREATE TABLE IF NOT EXISTS bronze_claude_enterprise.claude_enterprise_chat_projects
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `date` Nullable(String),
    `project_id` Nullable(String),
    `project_name` Nullable(String),
    `distinct_user_count` Nullable(Int64),
    `distinct_conversation_count` Nullable(Int64),
    `message_count` Nullable(Int64),
    `created_at` Nullable(String),
    `created_by_id` Nullable(String),
    `created_by_email` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_enterprise.claude_enterprise_connectors
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `date` Nullable(String),
    `connector_name` Nullable(String),
    `distinct_user_count` Nullable(Int64),
    `chat_conversation_connector_used_count` Nullable(Int64),
    `code_session_connector_used_count` Nullable(Int64),
    `excel_session_connector_used_count` Nullable(Int64),
    `powerpoint_session_connector_used_count` Nullable(Int64),
    `cowork_session_connector_used_count` Nullable(Int64),
    `surface_metrics_json` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_enterprise.claude_enterprise_skills
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `date` Nullable(String),
    `skill_name` Nullable(String),
    `distinct_user_count` Nullable(Int64),
    `chat_conversation_skill_used_count` Nullable(Int64),
    `code_session_skill_used_count` Nullable(Int64),
    `excel_session_skill_used_count` Nullable(Int64),
    `powerpoint_session_skill_used_count` Nullable(Int64),
    `cowork_session_skill_used_count` Nullable(Int64),
    `surface_metrics_json` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_enterprise.claude_enterprise_summaries
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `date` Nullable(String),
    `starting_at` Nullable(String),
    `ending_at` Nullable(String),
    `daily_active_user_count` Nullable(Int64),
    `weekly_active_user_count` Nullable(Int64),
    `monthly_active_user_count` Nullable(Int64),
    `daily_adoption_rate` Nullable(Decimal(38, 9)),
    `weekly_adoption_rate` Nullable(Decimal(38, 9)),
    `monthly_adoption_rate` Nullable(Decimal(38, 9)),
    `assigned_seat_count` Nullable(Int64),
    `pending_invite_count` Nullable(Int64),
    `cowork_daily_active_user_count` Nullable(Int64),
    `cowork_weekly_active_user_count` Nullable(Int64),
    `cowork_monthly_active_user_count` Nullable(Int64),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_enterprise.claude_enterprise_users
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `date` Nullable(String),
    `user_id` Nullable(String),
    `user_email` Nullable(String),
    `chat_conversation_count` Nullable(Int64),
    `chat_message_count` Nullable(Int64),
    `chat_projects_created_count` Nullable(Int64),
    `chat_projects_used_count` Nullable(Int64),
    `chat_files_uploaded_count` Nullable(Int64),
    `chat_artifacts_created_count` Nullable(Int64),
    `chat_thinking_message_count` Nullable(Int64),
    `chat_skills_used_count` Nullable(Int64),
    `chat_connectors_used_count` Nullable(Int64),
    `code_commit_count` Nullable(Int64),
    `code_pull_request_count` Nullable(Int64),
    `code_lines_added` Nullable(Int64),
    `code_lines_removed` Nullable(Int64),
    `code_session_count` Nullable(Int64),
    `code_tool_accepted_count` Nullable(Int64),
    `code_tool_rejected_count` Nullable(Int64),
    `web_search_count` Nullable(Int64),
    `excel_session_count` Nullable(Int64),
    `excel_message_count` Nullable(Int64),
    `powerpoint_session_count` Nullable(Int64),
    `powerpoint_message_count` Nullable(Int64),
    `cowork_session_count` Nullable(Int64),
    `cowork_message_count` Nullable(Int64),
    `cowork_action_count` Nullable(Int64),
    `cowork_dispatch_turn_count` Nullable(Int64),
    `cowork_skills_used_count` Nullable(Int64),
    `chat_metrics_json` Nullable(String),
    `claude_code_metrics_json` Nullable(String),
    `office_metrics_json` Nullable(String),
    `cowork_metrics_json` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

