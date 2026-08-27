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
    `source_entity_id` String,
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
    `source_entity_id` String,
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

CREATE TABLE IF NOT EXISTS insight.ci_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `source_entity_id` String,
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

CREATE TABLE IF NOT EXISTS insight.collab_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `source_entity_id` String,
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

CREATE TABLE IF NOT EXISTS insight.git_metric_evidence
(
    `tenant_id` String,
    `source_key` String,
    `entity_type` String,
    `entity_id` String,
    `source_entity_id` String,
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

CREATE TABLE IF NOT EXISTS insight.identity_resolution_coverage
(
    `source_key` String,
    `observation_rows` UInt64,
    `unresolved_rows` UInt64,
    `unresolved_people` UInt64,
    `match_rate_pct` Float64
)
ENGINE = MergeTree
ORDER BY source_key
SETTINGS replicated_deduplication_window = '0', index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS insight.metric_entity_cohorts_current
(
    `tenant_id` String,
    `entity_type` String,
    `entity_id` String,
    `cohort_key` String,
    `cohort_id` Nullable(String)
)
ENGINE = MergeTree
ORDER BY (tenant_id, entity_type, cohort_key, entity_id)
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
    `source_entity_id` String,
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
    `source_entity_id` String,
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

