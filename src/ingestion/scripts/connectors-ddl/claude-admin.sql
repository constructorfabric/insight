CREATE DATABASE IF NOT EXISTS `bronze_claude_admin`;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_api_keys
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `id` Nullable(String),
    `name` Nullable(String),
    `status` Nullable(String),
    `created_at` Nullable(String),
    `created_by_id` Nullable(String),
    `created_by_name` Nullable(String),
    `created_by_type` Nullable(String),
    `workspace_id` Nullable(String),
    `partial_key_hint` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_code_usage
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique` Nullable(String),
    `date` Nullable(String),
    `actor_type` Nullable(String),
    `actor_identifier` Nullable(String),
    `terminal_type` Nullable(String),
    `customer_type` Nullable(String),
    `session_count` Nullable(Decimal(38, 9)),
    `lines_added` Nullable(Decimal(38, 9)),
    `lines_removed` Nullable(Decimal(38, 9)),
    `tool_use_accepted` Nullable(Decimal(38, 9)),
    `tool_use_rejected` Nullable(Decimal(38, 9)),
    `core_metrics_json` Nullable(String),
    `tool_actions_json` Nullable(String),
    `model_breakdown_json` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_cost_report
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique` Nullable(String),
    `date` Nullable(String),
    `workspace_id` Nullable(String),
    `description` Nullable(String),
    `amount` Nullable(String),
    `currency` Nullable(String),
    `cost_type` Nullable(String),
    `model` Nullable(String),
    `service_tier` Nullable(String),
    `context_window` Nullable(String),
    `token_type` Nullable(String),
    `inference_geo` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_invites
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `id` Nullable(String),
    `email` Nullable(String),
    `role` Nullable(String),
    `status` Nullable(String),
    `created_at` Nullable(String),
    `expires_at` Nullable(String),
    `workspace_id` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_messages_usage
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique` Nullable(String),
    `date` Nullable(String),
    `model` Nullable(String),
    `api_key_id` Nullable(String),
    `workspace_id` Nullable(String),
    `service_tier` Nullable(String),
    `context_window` Nullable(String),
    `inference_geo` Nullable(String),
    `speed` Nullable(String),
    `uncached_input_tokens` Nullable(Decimal(38, 9)),
    `cache_read_tokens` Nullable(Decimal(38, 9)),
    `cache_creation_5m_tokens` Nullable(Decimal(38, 9)),
    `cache_creation_1h_tokens` Nullable(Decimal(38, 9)),
    `output_tokens` Nullable(Decimal(38, 9)),
    `web_search_requests` Nullable(Decimal(38, 9)),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_users
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `id` Nullable(String),
    `type` Nullable(String),
    `email` Nullable(String),
    `name` Nullable(String),
    `role` Nullable(String),
    `status` Nullable(String),
    `added_at` Nullable(String),
    `last_active_at` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_workspace_members
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique` Nullable(String),
    `type` Nullable(String),
    `user_id` Nullable(String),
    `workspace_id` Nullable(String),
    `workspace_role` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_claude_admin.claude_admin_workspaces
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `id` Nullable(String),
    `name` Nullable(String),
    `display_name` Nullable(String),
    `created_at` Nullable(String),
    `archived_at` Nullable(String),
    `data_residency` Nullable(String),
    `collected_at` Nullable(String),
    `data_source` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

