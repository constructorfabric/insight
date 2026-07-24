CREATE DATABASE IF NOT EXISTS `bronze_github_copilot`;

CREATE TABLE IF NOT EXISTS bronze_github_copilot.copilot_org_metrics
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `day` Nullable(String),
    `organization_id` Nullable(String),
    `enterprise_id` Nullable(String),
    `daily_active_users` Nullable(Decimal(38, 9)),
    `weekly_active_users` Nullable(Decimal(38, 9)),
    `monthly_active_users` Nullable(Decimal(38, 9)),
    `monthly_active_chat_users` Nullable(Decimal(38, 9)),
    `monthly_active_agent_users` Nullable(Decimal(38, 9)),
    `daily_active_copilot_cloud_agent_users` Nullable(Decimal(38, 9)),
    `weekly_active_copilot_cloud_agent_users` Nullable(Decimal(38, 9)),
    `monthly_active_copilot_cloud_agent_users` Nullable(Decimal(38, 9)),
    `daily_active_copilot_code_review_users` Nullable(Decimal(38, 9)),
    `weekly_active_copilot_code_review_users` Nullable(Decimal(38, 9)),
    `monthly_active_copilot_code_review_users` Nullable(Decimal(38, 9)),
    `daily_passive_copilot_code_review_users` Nullable(Decimal(38, 9)),
    `weekly_passive_copilot_code_review_users` Nullable(Decimal(38, 9)),
    `monthly_passive_copilot_code_review_users` Nullable(Decimal(38, 9)),
    `user_initiated_interaction_count` Nullable(Decimal(38, 9)),
    `code_generation_activity_count` Nullable(Decimal(38, 9)),
    `code_acceptance_activity_count` Nullable(Decimal(38, 9)),
    `loc_suggested_to_add_sum` Nullable(Decimal(38, 9)),
    `loc_suggested_to_delete_sum` Nullable(Decimal(38, 9)),
    `loc_added_sum` Nullable(Decimal(38, 9)),
    `loc_deleted_sum` Nullable(Decimal(38, 9)),
    `pull_requests` Nullable(String),
    `totals_by_ide` Nullable(String),
    `totals_by_feature` Nullable(String),
    `totals_by_language_feature` Nullable(String),
    `totals_by_language_model` Nullable(String),
    `totals_by_model_feature` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github_copilot.copilot_seats
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `user_login` Nullable(String),
    `user_email` Nullable(String),
    `plan_type` Nullable(String),
    `pending_cancellation_date` Nullable(String),
    `last_activity_at` Nullable(String),
    `last_activity_editor` Nullable(String),
    `last_authenticated_at` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `assignee` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github_copilot.copilot_user_metrics
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `day` Nullable(String),
    `user_login` Nullable(String),
    `user_id` Nullable(Decimal(38, 9)),
    `organization_id` Nullable(String),
    `enterprise_id` Nullable(String),
    `code_generation_activity_count` Nullable(Decimal(38, 9)),
    `code_acceptance_activity_count` Nullable(Decimal(38, 9)),
    `loc_suggested_to_add_sum` Nullable(Decimal(38, 9)),
    `loc_suggested_to_delete_sum` Nullable(Decimal(38, 9)),
    `loc_added_sum` Nullable(Decimal(38, 9)),
    `loc_deleted_sum` Nullable(Decimal(38, 9)),
    `user_initiated_interaction_count` Nullable(Decimal(38, 9)),
    `used_chat` Nullable(Bool),
    `used_agent` Nullable(Bool),
    `used_cli` Nullable(Bool),
    `used_copilot_coding_agent` Nullable(Bool),
    `used_copilot_cloud_agent` Nullable(Bool),
    `totals_by_ide` Nullable(String),
    `totals_by_feature` Nullable(String),
    `totals_by_language_feature` Nullable(String),
    `totals_by_language_model` Nullable(String),
    `totals_by_model_feature` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

