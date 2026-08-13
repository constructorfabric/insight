CREATE DATABASE IF NOT EXISTS `bronze_github`;

CREATE TABLE IF NOT EXISTS bronze_github.branches
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `repository` Nullable(String),
    `name` Nullable(String),
    `head_sha` Nullable(String),
    `head_committed_date` Nullable(String),
    `is_default` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.commits
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `repository` Nullable(String),
    `sha` Nullable(String),
    `message` Nullable(String),
    `authored_date` Nullable(String),
    `committed_date` Nullable(String),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `committer_name` Nullable(String),
    `committer_email` Nullable(String),
    `parent_hashes` Nullable(String),
    `is_merge` Nullable(Bool),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `changed_files` Nullable(Int64),
    `is_in_default_branch` Nullable(Bool),
    `patch_id` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.deployment_statuses
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `deployment_id` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `state` Nullable(String),
    `environment` Nullable(String),
    `creator_login` Nullable(String),
    `created_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.deployments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `sha` Nullable(String),
    `ref` Nullable(String),
    `task` Nullable(String),
    `environment` Nullable(String),
    `description` Nullable(String),
    `creator_login` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.file_changes
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `repository` Nullable(String),
    `sha` Nullable(String),
    `committed_date` Nullable(String),
    `filename` Nullable(String),
    `previous_filename` Nullable(String),
    `status` Nullable(String),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `changes` Nullable(Int64),
    `is_binary` Nullable(Bool),
    `patch` Nullable(String),
    `patch_truncated` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.issue_timeline_events
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `event_id` Nullable(String),
    `event_type` Nullable(String),
    `event_at` Nullable(String),
    `item_number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `actor_login` Nullable(String),
    `target_login` Nullable(String),
    `label_name` Nullable(String),
    `state_reason` Nullable(String),
    `field_name` Nullable(String),
    `prev_value` Nullable(String),
    `new_value` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.issues
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `state` Nullable(String),
    `state_reason` Nullable(String),
    `title` Nullable(String),
    `author_login` Nullable(String),
    `assignee_logins` Nullable(String),
    `label_names` Nullable(String),
    `comments` Nullable(Int64),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `closed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.projects_v2
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `project_id` Nullable(String),
    `number` Nullable(Int64),
    `org` Nullable(String),
    `title` Nullable(String),
    `short_description` Nullable(String),
    `public` Nullable(Bool),
    `closed` Nullable(Bool),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `issue_number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `author_login` Nullable(String),
    `author_association` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `body` Nullable(String),
    `author_id` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_commits
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `sha` Nullable(String),
    `pull_number` Nullable(Int64),
    `repo_full_name` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_diff_stats
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `pull_number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `changed_files` Nullable(Int64),
    `updated_at` Nullable(String),
    `author_email` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_review_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `pull_number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `author_login` Nullable(String),
    `author_association` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `body` Nullable(String),
    `author_id` Nullable(Int64),
    `path` Nullable(String),
    `line` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_reviews
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `pull_number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `author_login` Nullable(String),
    `author_association` Nullable(String),
    `state` Nullable(String),
    `commit_id` Nullable(String),
    `submitted_at` Nullable(String),
    `author_id` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_timeline_events
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `event_id` Nullable(String),
    `event_type` Nullable(String),
    `event_at` Nullable(String),
    `item_number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `actor_login` Nullable(String),
    `target_login` Nullable(String),
    `label_name` Nullable(String),
    `state_reason` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_requests
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `state` Nullable(String),
    `draft` Nullable(Bool),
    `title` Nullable(String),
    `author_login` Nullable(String),
    `author_association` Nullable(String),
    `head_sha` Nullable(String),
    `head_ref` Nullable(String),
    `base_ref` Nullable(String),
    `merge_commit_sha` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `closed_at` Nullable(String),
    `merged_at` Nullable(String),
    `body` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.repositories
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `full_name` Nullable(String),
    `name` Nullable(String),
    `org` Nullable(String),
    `default_branch` Nullable(String),
    `archived` Nullable(Bool),
    `fork` Nullable(Bool),
    `private` Nullable(Bool),
    `has_issues` Nullable(Bool),
    `has_wiki` Nullable(Bool),
    `language` Nullable(String),
    `size` Nullable(Int64),
    `clone_url` Nullable(String),
    `html_url` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `pushed_at` Nullable(String),
    `description` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.workflow_runs
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `run_attempt` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `name` Nullable(String),
    `workflow_id` Nullable(Int64),
    `event` Nullable(String),
    `status` Nullable(String),
    `conclusion` Nullable(String),
    `head_branch` Nullable(String),
    `head_sha` Nullable(String),
    `actor_login` Nullable(String),
    `run_started_at` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

