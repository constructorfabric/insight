CREATE DATABASE IF NOT EXISTS `bronze_bitbucket_cloud`;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.branches
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `name` Nullable(String),
    `target_hash` Nullable(String),
    `target_date` Nullable(String),
    `mainbranch_name` Nullable(String),
    `default_branch_name` Nullable(String),
    `is_default` Nullable(Bool),
    `updated_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.commit_branch_reachability
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `branch_name` Nullable(String),
    `branch_head_sha` Nullable(String),
    `default_branch_name` Nullable(String),
    `commit_sha` Nullable(String),
    `committed_at` Nullable(String),
    `reachability_action` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.commits
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `hash` Nullable(String),
    `message` Nullable(String),
    `date` Nullable(String),
    `author_raw` Nullable(String),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `author_display_name` Nullable(String),
    `author_uuid` Nullable(String),
    `author_account_id` Nullable(String),
    `committer_raw` Nullable(String),
    `committer_name` Nullable(String),
    `committer_email` Nullable(String),
    `committer_display_name` Nullable(String),
    `committer_uuid` Nullable(String),
    `committer_account_id` Nullable(String),
    `parent_hashes` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `branch_name` Nullable(String),
    `head_sha` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.deployments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.environments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.file_changes
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `source_type` Nullable(String),
    `sha` Nullable(String),
    `is_snapshot_marker` Nullable(Bool),
    `marker_type` Nullable(String),
    `filename` Nullable(String),
    `status` Nullable(String),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `previous_filename` Nullable(String),
    `committed_date` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.issue_changes
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `issue_id` Nullable(Int64),
    `issue_updated_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.issue_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `issue_id` Nullable(Int64),
    `issue_updated_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.issues
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pipeline_step_test_reports
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `pipeline_uuid` Nullable(String),
    `step_uuid` Nullable(String),
    `pipeline_created_on` Nullable(String),
    `step_completed_on` Nullable(String),
    `report` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pipeline_steps
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `created_on` Nullable(String),
    `completed_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pipelines
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `created_on` Nullable(String),
    `completed_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pull_request_activity
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `pr_id` Nullable(Int64),
    `event_type` Nullable(String),
    `activity_date` Nullable(String),
    `update_state` Nullable(String),
    `actor_display_name` Nullable(String),
    `actor_uuid` Nullable(String),
    `actor_account_id` Nullable(String),
    `pull_request_updated_on` Nullable(String),
    `pull_request_source_commit_hash` Nullable(String),
    `pull_request_destination_commit_hash` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pull_request_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `comment_id` Nullable(Int64),
    `pr_id` Nullable(Int64),
    `body` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String),
    `author_display_name` Nullable(String),
    `author_uuid` Nullable(String),
    `author_account_id` Nullable(String),
    `is_inline` Nullable(Bool),
    `inline_path` Nullable(String),
    `inline_from` Nullable(Int64),
    `inline_to` Nullable(Int64),
    `parent_comment_id` Nullable(Int64),
    `is_deleted` Nullable(Bool),
    `pull_request_updated_on` Nullable(String),
    `pull_request_source_commit_hash` Nullable(String),
    `pull_request_destination_commit_hash` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pull_request_commits
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `pr_id` Nullable(Int64),
    `hash` Nullable(String),
    `commit_order` Nullable(Int64),
    `author_uuid` Nullable(String),
    `author_account_id` Nullable(String),
    `pull_request_updated_on` Nullable(String),
    `pull_request_source_commit_hash` Nullable(String),
    `pull_request_destination_commit_hash` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pull_request_diffstat
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `pr_id` Nullable(Int64),
    `is_snapshot_marker` Nullable(Bool),
    `status` Nullable(String),
    `old_path` Nullable(String),
    `new_path` Nullable(String),
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `pull_request_updated_on` Nullable(String),
    `pull_request_source_commit_hash` Nullable(String),
    `pull_request_destination_commit_hash` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pull_request_tasks
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `pull_request_id` Nullable(Int64),
    `pull_request_updated_on` Nullable(String),
    `pull_request_source_commit_hash` Nullable(String),
    `pull_request_destination_commit_hash` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.pull_requests
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `id` Nullable(Int64),
    `title` Nullable(String),
    `description` Nullable(String),
    `state` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String),
    `author_display_name` Nullable(String),
    `author_uuid` Nullable(String),
    `author_account_id` Nullable(String),
    `closed_by_display_name` Nullable(String),
    `closed_by_uuid` Nullable(String),
    `closed_by_account_id` Nullable(String),
    `source_branch` Nullable(String),
    `destination_branch` Nullable(String),
    `source_commit_hash` Nullable(String),
    `destination_commit_hash` Nullable(String),
    `merge_commit_hash` Nullable(String),
    `task_count` Nullable(Int64),
    `draft` Nullable(Bool),
    `queued` Nullable(Bool),
    `close_source_branch` Nullable(Bool),
    `reason` Nullable(String),
    `reviewers` Nullable(String),
    `comment_count` Nullable(Int64),
    `participants` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.repositories
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `slug` Nullable(String),
    `name` Nullable(String),
    `full_name` Nullable(String),
    `uuid` Nullable(String),
    `is_private` Nullable(Bool),
    `description` Nullable(String),
    `language` Nullable(String),
    `size` Nullable(Int64),
    `created_on` Nullable(String),
    `updated_on` Nullable(String),
    `has_issues` Nullable(Bool),
    `has_wiki` Nullable(Bool),
    `mainbranch_name` Nullable(String),
    `scm` Nullable(String),
    `fork_policy` Nullable(String),
    `website` Nullable(String),
    `owner_uuid` Nullable(String),
    `owner_account_id` Nullable(String),
    `owner_display_name` Nullable(String),
    `owner_nickname` Nullable(String),
    `workspace_slug` Nullable(String),
    `parent_uuid` Nullable(String),
    `parent_full_name` Nullable(String),
    `project_key` Nullable(String),
    `project_name` Nullable(String),
    `project_uuid` Nullable(String),
    `links` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.tags
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `entity_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `record_type` Nullable(String),
    `generation_id` Nullable(String),
    `bucket_id` Nullable(Int64),
    `snapshot_item_count` Nullable(Int64),
    `snapshot_available` Nullable(Bool),
    `repository_uuid` Nullable(String),
    `workspace_uuid` Nullable(String),
    `workspace` Nullable(String),
    `repo_slug` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

