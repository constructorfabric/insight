CREATE DATABASE IF NOT EXISTS `bronze_bitbucket_cloud`;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.branches
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
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

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.commit_authors
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
    `repo_full_name` Nullable(String),
    `author_email` Nullable(String),
    `author_account_id` Nullable(String),
    `author_uuid` Nullable(String),
    `author_nickname` Nullable(String),
    `author_display_name` Nullable(String),
    `sample_sha` Nullable(String)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
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

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.deployments
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
    `uuid` Nullable(String),
    `repo_full_name` Nullable(String),
    `state_name` Nullable(String),
    `state_status` Nullable(String),
    `environment_uuid` Nullable(String),
    `deployable_commit_sha` Nullable(String),
    `deployable_pipeline_uuid` Nullable(String),
    `created_on` Nullable(String),
    `last_update_time` Nullable(String)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
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
    `patch_truncated` Nullable(Bool),
    `pre_image_oid` Nullable(String),
    `post_image_oid` Nullable(String)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `uuid` Nullable(String),
    `build_number` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `state_name` Nullable(String),
    `result_name` Nullable(String),
    `target_ref` Nullable(String),
    `target_commit_sha` Nullable(String),
    `trigger_name` Nullable(String),
    `creator_uuid` Nullable(String),
    `creator_account_id` Nullable(String),
    `creator_display_name` Nullable(String),
    `duration_in_seconds` Nullable(Int64),
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `pr_id` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `kind` Nullable(String),
    `event_date` Nullable(String),
    `actor_uuid` Nullable(String),
    `actor_display_name` Nullable(String),
    `update_state` Nullable(String),
    `update_source_commit` Nullable(String),
    `comment_id` Nullable(Int64),
    `actor_account_id` Nullable(String),
    `actor_nickname` Nullable(String),
    `changes` Nullable(String),
    `update_title` Nullable(String),
    `update_reason` Nullable(String),
    `update_draft` Nullable(Bool),
    `update_destination_commit` Nullable(String)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `pr_id` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `author_uuid` Nullable(String),
    `author_account_id` Nullable(String),
    `author_nickname` Nullable(String),
    `author_display_name` Nullable(String),
    `deleted` Nullable(Bool),
    `parent_id` Nullable(Int64),
    `inline_path` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String),
    `body` Nullable(String),
    `inline_to` Nullable(Int64),
    `inline_from` Nullable(Int64)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `sha` Nullable(String),
    `pr_id` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `author_email` Nullable(String),
    `author_name` Nullable(String),
    `author_account_id` Nullable(String),
    `author_uuid` Nullable(String),
    `author_nickname` Nullable(String),
    `author_display_name` Nullable(String),
    `message` Nullable(String),
    `committed_date` Nullable(String),
    `parent_shas` Nullable(String),
    `is_merge` Nullable(Bool),
    `pr_updated_on` Nullable(String)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `pr_id` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `file_path` Nullable(String),
    `old_path` Nullable(String),
    `status` Nullable(String),
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `pr_updated_on` Nullable(String)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `id` Nullable(Int64),
    `repo_full_name` Nullable(String),
    `title` Nullable(String),
    `state` Nullable(String),
    `draft` Nullable(Bool),
    `author_uuid` Nullable(String),
    `author_display_name` Nullable(String),
    `created_on` Nullable(String),
    `updated_on` Nullable(String),
    `merge_commit_sha` Nullable(String),
    `closed_by_uuid` Nullable(String),
    `source_branch` Nullable(String),
    `source_commit_sha` Nullable(String),
    `destination_branch` Nullable(String),
    `comment_count` Nullable(Int64),
    `task_count` Nullable(Int64),
    `description` Nullable(String),
    `destination_commit_sha` Nullable(String),
    `participants` Nullable(String),
    `author_account_id` Nullable(String),
    `author_nickname` Nullable(String),
    `closed_by_account_id` Nullable(String),
    `closed_by_display_name` Nullable(String),
    `source_repo_full_name` Nullable(String),
    `destination_repo_full_name` Nullable(String),
    `close_source_branch` Nullable(Bool),
    `reason` Nullable(String)
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
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `repository_uuid` Nullable(String),
    `clone_url` Nullable(String),
    `slug` Nullable(String),
    `name` Nullable(String),
    `full_name` Nullable(String),
    `is_private` Nullable(Bool),
    `has_issues` Nullable(Bool),
    `has_wiki` Nullable(Bool),
    `language` Nullable(String),
    `size` Nullable(Int64),
    `created_on` Nullable(String),
    `updated_on` Nullable(String),
    `description` Nullable(String),
    `default_branch` Nullable(String),
    `is_fork` Nullable(Bool),
    `parent_full_name` Nullable(String),
    `project_key` Nullable(String),
    `project_name` Nullable(String),
    `project_uuid` Nullable(String),
    `workspace_slug` Nullable(String),
    `workspace_uuid` Nullable(String),
    `owner_username` Nullable(String),
    `owner_uuid` Nullable(String),
    `fork_policy` Nullable(String),
    `enforced_signed_commits` Nullable(Bool),
    `scm` Nullable(String),
    `website` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bitbucket_cloud.workspace_members
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
    `account_id` Nullable(String),
    `display_name` Nullable(String),
    `nickname` Nullable(String),
    `user_uuid` Nullable(String),
    `workspace` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

