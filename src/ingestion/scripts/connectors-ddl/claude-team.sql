CREATE DATABASE IF NOT EXISTS `bronze_claude_team`;

CREATE TABLE IF NOT EXISTS bronze_claude_team.claude_team_code_metrics
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
    `metric_date` Nullable(String),
    `email` Nullable(String),
    `api_key_name` Nullable(String),
    `status` Nullable(String),
    `avg_cost_per_day` Nullable(String),
    `avg_lines_accepted_per_day` Nullable(Decimal(38, 9)),
    `total_cost` Nullable(String),
    `total_lines_accepted` Nullable(Decimal(38, 9)),
    `total_sessions` Nullable(Decimal(38, 9)),
    `last_active` Nullable(String),
    `prs_with_cc` Nullable(Decimal(38, 9)),
    `total_prs` Nullable(Decimal(38, 9)),
    `prs_with_cc_percentage` Nullable(Decimal(38, 9))
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_team.claude_team_invites
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
    `uuid` Nullable(String),
    `email_address` Nullable(String),
    `role` Nullable(String),
    `status` Nullable(String),
    `created_at` Nullable(String),
    `expires_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_team.claude_team_members
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
    `account` Nullable(String),
    `role` Nullable(String),
    `seat_tier` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_team.claude_team_overage_spend
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
    `account_uuid` Nullable(String),
    `account_email` Nullable(String),
    `account_name` Nullable(String),
    `seat_tier` Nullable(String),
    `is_enabled` Nullable(Bool),
    `monthly_credit_limit` Nullable(Decimal(38, 9)),
    `used_credits` Nullable(Decimal(38, 9)),
    `currency` Nullable(String),
    `out_of_credits` Nullable(Bool),
    `used_credits_basis` Nullable(String),
    `limit_type` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

