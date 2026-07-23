CREATE DATABASE IF NOT EXISTS `silver`;

CREATE TABLE IF NOT EXISTS silver.class_ai_api_usage
(
    `insight_tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `email` Nullable(String),
    `api_key_id` Nullable(String),
    `workspace_id` Nullable(String),
    `day` Nullable(Date),
    `provider` String,
    `channel` String,
    `input_tokens` UInt64,
    `output_tokens` UInt64,
    `cache_read_tokens` UInt64,
    `cache_creation_tokens` UInt64,
    `cost_amount` Nullable(Decimal(18, 4)),
    `cost_currency` Nullable(String),
    `source` String,
    `data_source` String,
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_ai_assistant_usage
(
    `insight_tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `email` Nullable(String),
    `day` Nullable(Date),
    `tool` String,
    `surface` String,
    `session_count` Nullable(UInt32),
    `conversation_count` Nullable(UInt32),
    `message_count` Nullable(UInt32),
    `action_count` Nullable(UInt32),
    `files_uploaded_count` Nullable(UInt32),
    `artifacts_created_count` Nullable(UInt32),
    `projects_created_count` Nullable(UInt32),
    `projects_used_count` Nullable(UInt32),
    `skills_used_count` Nullable(UInt32),
    `connectors_used_count` Nullable(UInt32),
    `thinking_message_count` Nullable(UInt32),
    `dispatch_turn_count` Nullable(UInt32),
    `search_count` Nullable(UInt32),
    `cost_cents` Nullable(UInt32),
    `surface_metrics_json` Nullable(String),
    `source` String,
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_ai_dev_usage
(
    `insight_tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `email` Nullable(String),
    `api_key_id` Nullable(String),
    `day` Nullable(Date),
    `tool` String,
    `session_count` UInt32,
    `conversation_count` Nullable(UInt32),
    `lines_added` UInt32,
    `lines_removed` Nullable(UInt32),
    `total_lines_added` Nullable(UInt32),
    `total_lines_removed` Nullable(UInt32),
    `tool_use_offered` Nullable(UInt32),
    `tool_use_accepted` Nullable(UInt32),
    `agent_sessions` Nullable(UInt32),
    `chat_requests` Nullable(UInt32),
    `cost_cents` Nullable(UInt32),
    `commits_count` Nullable(UInt32),
    `pull_requests_count` Nullable(UInt32),
    `prs_with_cc_count` Nullable(UInt32),
    `prs_total_count` Nullable(UInt32),
    `tool_action_breakdown_json` Nullable(String),
    `source` String,
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_ai_overage
(
    `insight_tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `email` Nullable(String),
    `account_id` Nullable(String),
    `period_month` Date,
    `tool` String,
    `seat_tier` Nullable(String),
    `currency` String,
    `credit_limit_cents` Nullable(UInt32),
    `used_amount_cents` UInt32,
    `overage_cents` Nullable(UInt32),
    `is_over_limit` Nullable(UInt8),
    `is_enabled` Nullable(UInt8),
    `overage_metrics_json` String,
    `source` String,
    `data_source` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_collab_chat_activity
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `user_id` Nullable(String),
    `user_name` Nullable(String),
    `email` Nullable(String),
    `person_key` Nullable(String),
    `date` Nullable(Date),
    `direct_messages` Nullable(Int64),
    `group_chat_messages` Nullable(Int64),
    `direct_and_group_messages` Nullable(Int64),
    `total_chat_messages` Int64,
    `channel_posts` Nullable(Int64),
    `channel_replies` Nullable(Int64),
    `urgent_messages` Nullable(Int64),
    `report_period` Nullable(String),
    `collected_at` DateTime,
    `data_source` String,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_collab_document_activity
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `user_id` Nullable(String),
    `user_name` Nullable(String),
    `email` Nullable(String),
    `person_key` Nullable(String),
    `date` Nullable(Date),
    `product` String,
    `viewed_or_edited_count` Nullable(Decimal(38, 9)),
    `synced_count` Nullable(Decimal(38, 9)),
    `shared_internally_count` Nullable(Decimal(38, 9)),
    `shared_externally_count` Nullable(Decimal(38, 9)),
    `visited_page_count` Nullable(Decimal(38, 9)),
    `report_period` Nullable(String),
    `collected_at` DateTime,
    `data_source` String,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_collab_email_activity
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `user_id` Nullable(String),
    `user_name` Nullable(String),
    `email` Nullable(String),
    `person_key` Nullable(String),
    `date` Nullable(Date),
    `sent_count` Nullable(Decimal(38, 9)),
    `received_count` Nullable(Decimal(38, 9)),
    `read_count` Nullable(Decimal(38, 9)),
    `meetings_created` Nullable(Decimal(38, 9)),
    `meetings_interacted` Nullable(Decimal(38, 9)),
    `report_period` Nullable(String),
    `collected_at` DateTime,
    `data_source` String,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_collab_meeting_activity
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `user_id` Nullable(String),
    `user_name` Nullable(String),
    `email` Nullable(String),
    `person_key` Nullable(String),
    `date` Nullable(Date),
    `calls_count` Nullable(Int64),
    `meetings_organized` Nullable(Int64),
    `meetings_attended` Int64,
    `adhoc_meetings_organized` Nullable(Int64),
    `adhoc_meetings_attended` Nullable(Int64),
    `scheduled_meetings_organized` Nullable(Int64),
    `scheduled_meetings_attended` Nullable(Int64),
    `audio_duration_seconds` Nullable(Int64),
    `video_duration_seconds` Nullable(Int64),
    `screen_share_duration_seconds` Nullable(Int64),
    `report_period` Nullable(String),
    `collected_at` DateTime,
    `data_source` String,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_crm_accounts
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `account_id` Nullable(String),
    `name` Nullable(String),
    `domain` Nullable(String),
    `industry` Nullable(String),
    `owner_id` Nullable(String),
    `parent_account_id` Nullable(String),
    `metadata` String,
    `custom_fields` Nullable(String),
    `created_at` Nullable(DateTime64(3)),
    `updated_at` Nullable(DateTime64(3)),
    `data_source` Nullable(String),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_crm_activities
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `activity_id` Nullable(String),
    `activity_type` String,
    `owner_id` Nullable(String),
    `created_by_user_id` Nullable(String),
    `contact_id` Nullable(String),
    `deal_id` Nullable(String),
    `account_id` Nullable(String),
    `timestamp` Nullable(DateTime64(3)),
    `duration_seconds` Nullable(Int64),
    `outcome` Nullable(String),
    `metadata` String,
    `custom_fields` Nullable(String),
    `created_at` Nullable(DateTime64(3)),
    `data_source` Nullable(String),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_crm_contacts
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `contact_id` Nullable(String),
    `email` Nullable(String),
    `first_name` Nullable(String),
    `last_name` Nullable(String),
    `owner_id` Nullable(String),
    `account_id` Nullable(String),
    `lifecycle_stage` Nullable(String),
    `metadata` String,
    `custom_fields` Nullable(String),
    `created_at` Nullable(DateTime64(3)),
    `updated_at` Nullable(DateTime64(3)),
    `data_source` Nullable(String),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_crm_deals
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `deal_id` Nullable(String),
    `name` Nullable(String),
    `forecast_category` Nullable(String),
    `stage` Nullable(String),
    `amount` Nullable(Float64),
    `amount_home` Nullable(Float64),
    `acv` Nullable(Float64),
    `tcv` Nullable(Float64),
    `arr` Nullable(Float64),
    `close_date` Nullable(Date32),
    `owner_id` Nullable(String),
    `created_by_user_id` Nullable(String),
    `account_id` Nullable(String),
    `is_closed` Nullable(Int64),
    `is_won` Nullable(Int64),
    `lead_source` Nullable(String),
    `probability` Nullable(Float64),
    `deal_type` Nullable(String),
    `lost_reason` Nullable(String),
    `pipeline_id` Nullable(String),
    `metadata` String,
    `custom_fields` Nullable(String),
    `created_at` Nullable(DateTime64(3)),
    `updated_at` Nullable(DateTime64(3)),
    `data_source` Nullable(String),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_crm_users
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `user_id` Nullable(String),
    `hs_user_id` Nullable(String),
    `email` Nullable(String),
    `first_name` Nullable(String),
    `last_name` Nullable(String),
    `title` Nullable(String),
    `department` Nullable(String),
    `is_active` Nullable(Int64),
    `metadata` String,
    `custom_fields` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `data_source` Nullable(String),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_focus_metrics
(
    `insight_tenant_id` Nullable(String),
    `email` Nullable(String),
    `day` Nullable(Date),
    `unique_key` Nullable(String),
    `meetings_count` Int64,
    `meeting_hours` Nullable(Float64),
    `working_hours_per_day` Float64,
    `focus_time_pct` Nullable(Float64),
    `dev_time_h` Nullable(Float64),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_git_pull_requests_comments
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `pr_id` Int64,
    `comment_id` Int64,
    `content` String,
    `author_name` String,
    `author_uuid` String,
    `created_at` Nullable(DateTime),
    `updated_at` Nullable(DateTime),
    `is_inline` UInt8,
    `file_path` String,
    `line_number` Int64,
    `data_source` String,
    `_version` Int64,
    `_airbyte_extracted_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_git_pull_requests_commits
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `pr_id` Int64,
    `commit_hash` String,
    `commit_order` Int64,
    `data_source` String,
    `_version` Int64,
    `_airbyte_extracted_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_git_pull_requests_reviewers
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `pr_id` Int64,
    `reviewer_name` String,
    `reviewer_uuid` String,
    `status` String,
    `approved` UInt8,
    `reviewed_at` Nullable(DateTime),
    `data_source` String,
    `_version` Int64,
    `_airbyte_extracted_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_git_repositories
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `repo_uuid` String,
    `name` String,
    `full_name` String,
    `description` String,
    `is_private` UInt8,
    `created_on` Nullable(DateTime),
    `updated_on` Nullable(DateTime),
    `size` Int64,
    `language` String,
    `has_issues` UInt8,
    `has_wiki` UInt8,
    `metadata` String,
    `data_source` String,
    `_version` Int64,
    `_airbyte_extracted_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_git_repository_branches
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `branch_name` String,
    `is_default` UInt8,
    `last_commit_hash` String,
    `last_commit_date` Nullable(DateTime),
    `data_source` String,
    `_version` Int64,
    `_airbyte_extracted_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_hr_events
(
    `insight_tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `source_person_id` Nullable(String),
    `email` Nullable(String),
    `event_type` String,
    `event_subtype` Nullable(String),
    `start_date` Nullable(DateTime),
    `end_date` Nullable(DateTime),
    `duration_amount` Nullable(Float64),
    `duration_unit` Nullable(String),
    `request_status` Nullable(String),
    `source` String,
    `created_at` Nullable(DateTime),
    `ingested_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_hr_working_hours
(
    `insight_tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `source_person_id` Nullable(String),
    `email` Nullable(String),
    `display_name` Nullable(String),
    `employment_type` Nullable(String),
    `source` String,
    `working_hours_per_day` Float64,
    `working_hours_per_week` Float64,
    `ingested_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_people
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `workspace_id` String,
    `person_id` Nullable(UUID),
    `valid_from` Nullable(DateTime),
    `valid_to` Nullable(DateTime),
    `source` String,
    `source_person_id` Nullable(String),
    `employee_number` Nullable(String),
    `display_name` Nullable(String),
    `first_name` Nullable(String),
    `last_name` Nullable(String),
    `email` Nullable(String),
    `job_title` Nullable(String),
    `department_name` Nullable(String),
    `org_unit_id` Nullable(UUID),
    `manager_person_id` Nullable(String),
    `status` String,
    `employment_type` String,
    `hire_date` Nullable(DateTime),
    `termination_date` Nullable(DateTime),
    `location` Nullable(String),
    `country` Nullable(String),
    `fte` Nullable(Float64),
    `custom_str_attrs` Map(String, String),
    `custom_num_attrs` Map(String, Float64),
    `ingested_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_support_activity
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `data_source` String,
    `person_key` Nullable(String),
    `email` Nullable(String),
    `date` Nullable(Date),
    `updates` UInt64,
    `public_comments` UInt64,
    `private_comments` UInt64,
    `solved` UInt64,
    `kb_articles_created` Nullable(UInt32),
    `csat_good` UInt64,
    `csat_total` UInt64,
    `collected_at` DateTime,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_comments
(
    `unique_key` Nullable(String),
    `insight_source_id` Nullable(String),
    `data_source` String,
    `comment_id` Nullable(String),
    `id_readable` Nullable(String),
    `author_id` Nullable(String),
    `created_at` Nullable(DateTime64(3)),
    `updated_at` Nullable(DateTime64(3)),
    `body` Nullable(String),
    `is_deleted` Nullable(UInt8),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_field_history
(
    `unique_key` String,
    `insight_source_id` String,
    `data_source` String,
    `issue_id` String,
    `id_readable` String,
    `event_id` String,
    `event_at` DateTime64(3),
    `event_kind` Enum8('changelog' = 1, 'synthetic_initial' = 2),
    `_seq` UInt32,
    `author_id` Nullable(String),
    `author_display` Nullable(String),
    `field_id` String,
    `field_name` String,
    `field_cardinality` Enum8('single' = 1, 'multi' = 2),
    `delta_action` Enum8('set' = 1, 'add' = 2, 'remove' = 3),
    `delta_value_id` Nullable(String),
    `delta_value_display` Nullable(String),
    `value_ids` Array(String),
    `value_displays` Array(String),
    `value_id_type` Enum8('opaque_id' = 1, 'account_id' = 2, 'string_literal' = 3, 'path' = 4, 'none' = 5),
    `collected_at` DateTime64(3),
    `_version` UInt64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_field_metadata
(
    `unique_key` Nullable(String),
    `insight_source_id` Nullable(String),
    `data_source` String,
    `project_key` Nullable(String),
    `field_id` Nullable(String),
    `field_name` Nullable(String),
    `is_multi` Nullable(UInt8),
    `field_type` Nullable(String),
    `has_id` Nullable(UInt8),
    `observed_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_projects
(
    `unique_key` Nullable(String),
    `insight_source_id` Nullable(String),
    `data_source` String,
    `project_id` Nullable(String),
    `project_key` Nullable(String),
    `name` Nullable(String),
    `lead_id` Nullable(String),
    `project_type` Nullable(String),
    `project_style` Nullable(String),
    `archived` Nullable(UInt8),
    `collected_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_sprints
(
    `unique_key` Nullable(String),
    `insight_source_id` Nullable(String),
    `data_source` String,
    `sprint_id` Nullable(String),
    `board_id` Nullable(String),
    `board_name` Nullable(String),
    `sprint_name` Nullable(String),
    `project_key` Nullable(String),
    `state` Nullable(String),
    `start_date` Nullable(DateTime64(3)),
    `end_date` Nullable(DateTime64(3)),
    `complete_date` Nullable(DateTime64(3)),
    `collected_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_statuses
(
    `unique_key` Nullable(String),
    `insight_source_id` Nullable(String),
    `data_source` String,
    `status_id` Nullable(String),
    `status_name` Nullable(String),
    `category_id` Nullable(Int32),
    `category_key` Nullable(String),
    `status_category` String,
    `collected_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_users
(
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `data_source` String,
    `user_id` Nullable(String),
    `email` Nullable(String),
    `display_name` Nullable(String),
    `username` Nullable(String),
    `account_type` Nullable(String),
    `is_active` Nullable(UInt8),
    `collected_at` DateTime64(3),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_task_worklogs
(
    `unique_key` Nullable(String),
    `insight_source_id` Nullable(String),
    `data_source` String,
    `worklog_id` Nullable(String),
    `id_readable` Nullable(String),
    `author_id` Nullable(String),
    `work_date` Nullable(DateTime64(3)),
    `duration_seconds` Nullable(Float64),
    `description` Nullable(String),
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_wiki_activity
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `author_id` Nullable(String),
    `author_email` Nullable(String),
    `day` Nullable(Date),
    `pages_edited` UInt32,
    `total_edits` UInt32,
    `pages_created` UInt32,
    `source` String,
    `data_source` String,
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_wiki_engagement
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String,
    `page_id` Nullable(String),
    `day` Nullable(Date),
    `total_comments` UInt32,
    `footer_comments` UInt32,
    `inline_comments` UInt32,
    `replies` UInt32,
    `unique_commenters` UInt32,
    `unresolved_inline_count` UInt32,
    `source` String,
    `data_source` String,
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.class_wiki_pages
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `page_id` Nullable(String),
    `space_id` Nullable(String),
    `space_name` Nullable(String),
    `title` Nullable(String),
    `status` Nullable(String),
    `author_id` Nullable(String),
    `author_email` Nullable(String),
    `last_editor_id` Nullable(String),
    `last_editor_email` Nullable(String),
    `parent_page_id` Nullable(String),
    `version_count` UInt32,
    `created_at` Nullable(DateTime64(3)),
    `updated_at` Nullable(DateTime64(3)),
    `space_url` Nullable(String),
    `source` String,
    `data_source` String,
    `collected_at` Nullable(DateTime64(3)),
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.dim_support_agent
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `data_source` String,
    `source_agent_id` Nullable(String),
    `person_key` Nullable(String),
    `email` Nullable(String),
    `display_name` Nullable(String),
    `role_canonical` String,
    `group_source_id` Nullable(String),
    `group_name` Nullable(String),
    `is_active` Nullable(Int64),
    `collected_at` DateTime,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.dim_support_ticket
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(FixedString(16)),
    `data_source` String,
    `source_ticket_id` Nullable(String),
    `subject` Nullable(String),
    `status_canonical` Nullable(String),
    `priority_canonical` String,
    `type_canonical` String,
    `assignee_source_id` Nullable(String),
    `assignee_person_key` Nullable(String),
    `group_source_id` Nullable(String),
    `requester_source_id` Nullable(String),
    `org_source_id` Nullable(String),
    `created_at` Nullable(DateTime),
    `updated_at` Nullable(DateTime),
    `solved_at` Nullable(DateTime),
    `collected_at` DateTime,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.fct_git_review
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `pr_id` Int64,
    `reviewer_name` String,
    `reviewer_uuid` String,
    `person_key` String,
    `status` String,
    `approved` UInt8,
    `reviewed_at` Nullable(DateTime),
    `data_source` String,
    `_version` Int64,
    `_airbyte_extracted_at` DateTime64(3)
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.to_ai_cost
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_id` Nullable(String),
    `report_date` Date,
    `line_item` Nullable(String),
    `project_id` Nullable(String),
    `amount_value` Nullable(Decimal(38, 9)),
    `amount_currency` Nullable(String),
    `provider` String,
    `data_source` String
)
ENGINE = MergeTree
ORDER BY tuple()
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.to_ai_tool_usage
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_id` Nullable(String),
    `report_date` Date,
    `user_id` Nullable(String),
    `project_id` Nullable(String),
    `model` Nullable(String),
    `input_tokens` Nullable(Decimal(38, 9)),
    `output_tokens` Nullable(Decimal(38, 9)),
    `input_cached_tokens` Nullable(Decimal(38, 9)),
    `input_audio_tokens` Nullable(Decimal(38, 9)),
    `output_audio_tokens` Nullable(Decimal(38, 9)),
    `num_model_requests` Nullable(Decimal(38, 9)),
    `is_batch` Bool,
    `service_tier` Nullable(String),
    `person_id` Nullable(String),
    `provider` String,
    `client` String,
    `data_source` String
)
ENGINE = MergeTree
ORDER BY tuple()
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS silver.zendesk__support_event
(
    `tenant_id` Nullable(String),
    `insight_source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` String,
    `ticket_key` Nullable(String),
    `source_ticket_id` Nullable(String),
    `actor_person_key` Nullable(String),
    `actor_source_id` Nullable(String),
    `event_type` String,
    `is_public` Nullable(UInt8),
    `occurred_at` Nullable(DateTime),
    `metric_date` Nullable(Date),
    `collected_at` DateTime,
    `_version` Int64
)
ENGINE = ReplacingMergeTree(_version)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

