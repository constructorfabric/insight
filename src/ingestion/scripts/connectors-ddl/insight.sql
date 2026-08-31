CREATE DATABASE IF NOT EXISTS `insight`;

CREATE TABLE IF NOT EXISTS insight.account_attribute_values
(
    `insight_tenant_id` String,
    `insight_source_type` String,
    `insight_source_id` String,
    `source_account_id` String,
    `field_id` String,
    `value_id` Nullable(String),
    `value_label` String,
    `valid_from` DateTime64(3),
    `valid_to` Nullable(DateTime64(3)),
    `ingested_at` DateTime64(3)
)
ENGINE = MergeTree
ORDER BY (insight_tenant_id, insight_source_type, insight_source_id, source_account_id, field_id, valid_from)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_cost_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_cost_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_seat_days
(
    `tenant_id` String,
    `source_id` Nullable(String),
    `account_id` Nullable(String),
    `email` String,
    `snapshot_date` Date,
    `tool` String,
    `tool_label` String,
    `seat_tier` String,
    `daily_extra_usage_usd` Float64
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(snapshot_date)
ORDER BY (tenant_id, email, snapshot_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_seat_months
(
    `tenant_id` String,
    `source_id` Nullable(String),
    `account_id` Nullable(String),
    `email` String,
    `period_month` Date,
    `tool` String,
    `tool_label` String,
    `seat_tier` String,
    `extra_usage_usd` Float64,
    `extra_usage_limit_usd` Nullable(Float64),
    `seat_cost_usd` Nullable(Float64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(period_month)
ORDER BY (tenant_id, email, period_month)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ai_usage
(
    `tenant_id` String,
    `email` String,
    `usage_date` Date,
    `tool` String,
    `tool_label` String,
    `surface` String,
    `surface_label` String,
    `seat_status` String,
    `lines_added` Nullable(UInt64),
    `lines_removed` Nullable(UInt64),
    `tool_use_offered` Nullable(UInt64),
    `tool_use_accepted` Nullable(UInt64),
    `dev_conversations` Nullable(UInt64),
    `assistant_messages` Nullable(UInt64),
    `assistant_actions` Nullable(UInt64),
    `chat_conversations` Nullable(UInt64),
    `prs_with_assistant` Nullable(UInt64),
    `prs_total` Nullable(UInt64),
    `cost_usd` Nullable(Float64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(usage_date)
ORDER BY (tenant_id, email, usage_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ci_commits
(
    `tenant_id` String,
    `entity_id` String,
    `source_id` String,
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String,
    `metric_date` Date,
    `committed_at` DateTime64(3),
    `commit_reference` String,
    `repository_value` String,
    `repository_label` String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ci_deployments
(
    `tenant_id` String,
    `entity_id` String,
    `source_id` String,
    `deployment_id` String,
    `metric_date` Date,
    `created_at` DateTime64(3),
    `deployment_label` String,
    `repository_value` String,
    `repository_label` String,
    `environment_value` String,
    `environment_label` String,
    `outcome_value` String,
    `outcome_label` String,
    `env_kind_value` String,
    `env_kind_label` String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ci_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `source_entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, Nullable(String))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ci_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.ci_runs
(
    `tenant_id` String,
    `entity_id` String,
    `source_id` String,
    `run_id` String,
    `run_number` String,
    `metric_date` Date,
    `started_at` DateTime64(3),
    `is_gate` UInt8,
    `is_retry` UInt8,
    `attempt` UInt32,
    `commit_known` UInt8,
    `branch` String,
    `duration_min` Float64,
    `duration_h` Float64,
    `run_label` String,
    `repository_value` String,
    `repository_label` String,
    `pipeline_value` String,
    `pipeline_label` String,
    `trigger_value` String,
    `trigger_label` String,
    `outcome_value` String,
    `outcome_label` String,
    `hour_block_value` String,
    `hour_block_label` String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.collab_active_modalities
(
    `tenant_id` String,
    `person_email` String,
    `activity_date` Date,
    `tool` String,
    `modality` String
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(activity_date)
ORDER BY (tenant_id, person_email, activity_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.collab_activity
(
    `tenant_id` String,
    `person_email` String,
    `activity_date` Date,
    `tool` String,
    `tool_label` String,
    `total_chat_messages` Nullable(Int64),
    `channel_posts_total` Nullable(Int64),
    `direct_and_group_messages` Nullable(Int64),
    `emails_sent` Nullable(Float64),
    `emails_received` Nullable(Float64),
    `emails_read` Nullable(Float64),
    `files_engaged` Nullable(Float64),
    `files_shared_internal` Nullable(Float64),
    `files_shared_external` Nullable(Float64),
    `meeting_seconds` Nullable(Int64),
    `meeting_hours` Nullable(Float64),
    `meetings_attended` Nullable(Int64),
    `meetings_organized` Nullable(Int64),
    `adhoc_meetings_attended` Nullable(Int64),
    `scheduled_meetings_attended` Nullable(Int64),
    `focus_hours` Nullable(Float64),
    `working_hours` Nullable(Float64),
    `is_chat_active` UInt8,
    `is_email_active` UInt8,
    `is_documents_active` UInt8,
    `is_meetings_active` UInt8,
    `is_deliberately_active` UInt8
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(activity_date)
ORDER BY (tenant_id, person_email, activity_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.collab_file_shares
(
    `tenant_id` String,
    `person_email` String,
    `activity_date` Date,
    `tool` String,
    `scope` String,
    `scope_label` String,
    `files_shared` Nullable(Float64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(activity_date)
ORDER BY (tenant_id, person_email, activity_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.collab_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.collab_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_authored_commits
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String,
    `data_source` String,
    `author_name` String,
    `message` String,
    `observed_at` Nullable(DateTime),
    `entity_id` String,
    `metric_date` Nullable(Date),
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `branch_scope_value` String,
    `branch_scope_label` String,
    `project_value` String,
    `project_label` String,
    `repository_value` String,
    `repository_label` String,
    `source_value` String,
    `source_label` String,
    `source_dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
ORDER BY (tenant_id, data_source, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_commit_file_changes
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String,
    `observed_at` Nullable(DateTime),
    `data_source` String,
    `file_path` String,
    `file_extension` String,
    `change_type` String,
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `pre_image_oid` Nullable(String),
    `post_image_oid` Nullable(String)
)
ENGINE = MergeTree
ORDER BY (tenant_id, data_source, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_commits
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String,
    `author_email` String,
    `author_name` String,
    `message` String,
    `authored_at` DateTime,
    `authored_date` Date,
    `branch_scope` String,
    `branch_scope_label` String,
    `repository` String,
    `repository_label` String,
    `project` String,
    `project_label` String,
    `source` String,
    `source_label` String,
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(authored_date)
ORDER BY (tenant_id, author_email, authored_at)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_default_branch_commits
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String
)
ENGINE = MergeTree
ORDER BY (tenant_id, source_id, project_key, repo_slug, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_derived_commits
(
    `tenant_id` Nullable(String),
    `data_source` String,
    `commit_hash` String,
    `reason` String
)
ENGINE = MergeTree
ORDER BY (tenant_id, data_source, commit_hash)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_file_changes
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `commit_hash` String,
    `file_path` String,
    `author_email` String,
    `author_name` String,
    `authored_at` DateTime,
    `authored_date` Date,
    `category` String,
    `category_label` String,
    `file_extension` String,
    `file_extension_label` String,
    `change_type` String,
    `change_type_label` String,
    `branch_scope` String,
    `branch_scope_label` String,
    `repository` String,
    `repository_label` String,
    `project` String,
    `project_label` String,
    `source` String,
    `source_label` String,
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(authored_date)
ORDER BY (tenant_id, author_email, authored_at)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_pull_requests
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `project_key` String,
    `repo_slug` String,
    `pr_id` Int64,
    `pr_number` Int64,
    `title` String,
    `author_email` String,
    `author_account_id` String,
    `author_name` String,
    `created_at` DateTime,
    `created_date` Date,
    `closed_at` Nullable(DateTime),
    `merged` UInt8,
    `abandoned` UInt8,
    `merged_without_approval` UInt8,
    `reviewer_count` UInt64,
    `first_reviewed_at` Nullable(DateTime),
    `approved_at` Nullable(DateTime),
    `branch_scope` String,
    `branch_scope_label` String,
    `destination_branch` String,
    `destination_branch_label` String,
    `repository` String,
    `repository_label` String,
    `project` String,
    `project_label` String,
    `source` String,
    `source_label` String,
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `files_changed` Nullable(Int64),
    `linked_commit_count` Nullable(UInt64),
    `cycle_hours` Nullable(Float64),
    `first_review_hours` Nullable(Float64),
    `review_to_merge_hours` Nullable(Float64),
    `approval_to_merge_hours` Nullable(Float64),
    `review_wait_share` Nullable(Float64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(created_date)
ORDER BY (tenant_id, author_email, created_at)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.git_review_events
(
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `event_kind` String,
    `event_key` Nullable(String),
    `pr_id` Int64,
    `pr_number` Int64,
    `entity_id` String,
    `metric_date` Nullable(Date),
    `observed_at` Nullable(DateTime),
    `title` String,
    `author_name` String,
    `author_email` String,
    `actor_person_id` String,
    `author_person_id` String,
    `comment_target_value` String,
    `comment_target_label` String,
    `project_value` String,
    `project_label` String,
    `repository_value` String,
    `repository_label` String,
    `destination_branch_value` String,
    `destination_branch_label` String,
    `source_value` String,
    `source_label` String,
    `source_dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
ORDER BY (tenant_id, entity_id, metric_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_close_events
(
    `tenant_id` String,
    `insight_source_id` String,
    `issue_id` String,
    `id_readable` String,
    `title` Nullable(String),
    `assignee_email` String,
    `close_at` DateTime64(3),
    `close_date` Date,
    `reopened_within_14d` UInt8
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(close_date)
ORDER BY (tenant_id, assignee_email, close_at)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_closed_issues
(
    `tenant_id` String,
    `insight_source_id` String,
    `issue_id` String,
    `id_readable` String,
    `title` Nullable(String),
    `assignee_email` String,
    `closed_at` DateTime64(3),
    `closed_date` Date,
    `issue_type` String,
    `issue_type_label` String,
    `source` String,
    `source_label` String,
    `is_closed` UInt8,
    `is_bug` UInt8,
    `is_non_bug` UInt8,
    `has_due_date` UInt8,
    `on_time` UInt8,
    `is_late` UInt8,
    `slip_days` Nullable(Float64),
    `dev_seconds` Float64,
    `dev_hours` Float64,
    `lead_seconds` Float64,
    `resolution_days` Float64,
    `pickup_days` Nullable(Float64),
    `estimate_seconds` Nullable(Float64),
    `spent_seconds` Nullable(Float64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(closed_date)
ORDER BY (tenant_id, assignee_email, closed_at)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_estimation_days
(
    `tenant_id` String,
    `assignee_email` String,
    `closed_date` Date,
    `estimation_pct` Nullable(Float64),
    `estimation_error_pct` Nullable(Float64)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(closed_date)
ORDER BY (tenant_id, assignee_email, closed_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_issue_state
(
    `tenant_id` Nullable(String),
    `entity_id` Nullable(String),
    `insight_source_id` String,
    `data_source` String,
    `issue_id` String,
    `id_readable` String,
    `title` Nullable(String),
    `status_category` String,
    `issue_type` String,
    `issue_kind` String,
    `issue_type_key` Nullable(String),
    `issue_type_name` Nullable(String),
    `due_date` Nullable(Date),
    `time_estimate_seconds` Nullable(Float64),
    `time_spent_seconds` Nullable(Float64),
    `created_at` DateTime64(3),
    `final_close_at` Nullable(DateTime64(3)),
    `last_status_event_at` DateTime64(3)
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_open_issues
(
    `tenant_id` String,
    `insight_source_id` String,
    `issue_id` String,
    `id_readable` String,
    `title` Nullable(String),
    `assignee_email` String,
    `last_status_event_at` DateTime64(3),
    `last_status_event_date` Date,
    `idle_days` Int64
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(last_status_event_date)
ORDER BY (tenant_id, assignee_email, last_status_event_at)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_status_spans
(
    `insight_source_id` String,
    `issue_id` String,
    `interval_start` DateTime64(3),
    `interval_end` DateTime64(3),
    `status_category` String,
    `duration_seconds` Float64
)
ENGINE = MergeTree
ORDER BY (insight_source_id, issue_id, interval_start)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.task_worklog_flow
(
    `tenant_id` String,
    `entity_id` String,
    `metric_date` Date,
    `in_progress_seconds` Float64,
    `worklog_seconds` Float64
)
ENGINE = MergeTree
ORDER BY (tenant_id, entity_id, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.wiki_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `record_id` String,
    `record_kind` String,
    `granularity` String,
    `record_label` String,
    `contribution` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String))),
    `details` Map(String, String)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date, record_id)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.wiki_metric_observations
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `account_source_type` String,
    `account_source_id` String,
    `account_id` String,
    `metric_date` Date,
    `observed_at` Nullable(DateTime64(3)),
    `measure_key` String,
    `value` Nullable(Float64),
    `subject_key` Nullable(String),
    `dimensions` Array(Tuple(
        key String,
        value String,
        label Nullable(String)))
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(metric_date)
ORDER BY (tenant_id, source_key, entity_type, entity_id, measure_key, metric_date)
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.wiki_pages
(
    `tenant_id` String,
    `source_id` Nullable(String),
    `page_id` String,
    `title` Nullable(String),
    `space_name` Nullable(String),
    `author_email` String,
    `created_at` Nullable(DateTime64(3)),
    `created_date` Nullable(Date)
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(created_date)
ORDER BY (tenant_id, author_email, created_at)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.wiki_person_days
(
    `tenant_id` String,
    `source_id` Nullable(String),
    `author_email` String,
    `activity_date` Date,
    `edits` UInt64,
    `pages_edited` UInt64,
    `comments_received` UInt64
)
ENGINE = MergeTree
PARTITION BY toYYYYMM(activity_date)
ORDER BY (tenant_id, author_email, activity_date)
SETTINGS allow_nullable_key = 1, replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE OR REPLACE VIEW insight.identity_resolution_coverage
(
    `source_key` String,
    `observation_rows` UInt64,
    `unresolved_rows` UInt64,
    `unresolved_people` UInt64,
    `match_rate_pct` Float64
)
AS WITH
    source_identities AS
    (
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.git_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.ai_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.collab_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.task_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.wiki_metric_evidence
        UNION ALL
        SELECT
            source_key,
            entity_id AS email,
            account_source_type,
            account_source_id,
            account_id
        FROM insight.ai_cost_metric_evidence
        UNION ALL
        SELECT DISTINCT
            'hr_cohorts' AS source_key,
            lower(trimBoth(assumeNotNull(email))) AS email,
            '' AS account_source_type,
            '' AS account_source_id,
            '' AS account_id
        FROM silver.class_people
        FINAL
        WHERE (email IS NOT NULL) AND (email != '')
    ),
    resolution AS
    (
        SELECT
            source_identities.source_key AS source_key,
            source_identities.email AS email,
            source_identities.account_id AS account_id,
            (coalesce(account_map.account_id, '') != '') OR (coalesce(person_map.email, '') != '') AS resolved
        FROM source_identities
        LEFT JOIN identity.account_assignment AS account_map ON (account_map.source_type = source_identities.account_source_type) AND (account_map.source_id = toUUID(UUIDNumToString(sipHash128(coalesce(source_identities.account_source_id, ''))))) AND (account_map.account_id = lower(trimBoth(coalesce(source_identities.account_id, ''))))
        LEFT JOIN identity.person_map AS person_map ON person_map.email = source_identities.email
    )
SELECT
    source_key,
    count() AS observation_rows,
    countIf(NOT resolved) AS unresolved_rows,
    uniqExactIf(if(email != '', email, coalesce(account_id, '')), NOT resolved) AS unresolved_people,
    round((100 * countIf(resolved)) / count(), 1) AS match_rate_pct
FROM resolution
GROUP BY source_key
;

CREATE OR REPLACE VIEW insight.metric_entity_cohorts_current
(
    `tenant_id` String,
    `entity_type` String,
    `entity_id` String,
    `cohort_key` String,
    `cohort_id` Nullable(String)
)
AS SELECT
    tenant_id,
    entity_type,
    entity_id,
    cohort_key,
    resolved_cohort_id AS cohort_id
FROM
(
    SELECT
        tenant_id,
        entity_type,
        entity_id,
        cohort_key,
        any(cohort_id) AS resolved_cohort_id
    FROM
    (
        SELECT
            assumeNotNull(people.tenant_id) AS tenant_id,
            'person' AS entity_type,
            toString(person_map.person_id) AS entity_id,
            'org_unit' AS cohort_key,
            people.cohort_id AS cohort_id
        FROM
        (
            SELECT
                workspace_id AS tenant_id,
                lower(trimBoth(assumeNotNull(email))) AS email,
                nullIf(department_name, '') AS cohort_id
            FROM silver.class_people
            WHERE (email IS NOT NULL) AND (email != '') AND (workspace_id IS NOT NULL) AND (workspace_id != '')
            ORDER BY
                tenant_id ASC,
                email ASC,
                coalesce(parseDateTimeBestEffortOrNull(toString(valid_from)), toDateTime('1970-01-01')) DESC,
                unique_key DESC
            LIMIT 1 BY
                tenant_id,
                email
        ) AS people
        INNER JOIN identity.person_map AS person_map ON person_map.email = people.email
        WHERE (people.tenant_id IS NOT NULL) AND (people.tenant_id != '') AND (people.email != '') AND (people.cohort_id IS NOT NULL)
    ) AS resolved
    GROUP BY
        tenant_id,
        entity_type,
        entity_id,
        cohort_key
    HAVING uniqExact(cohort_id) = 1
)
;

