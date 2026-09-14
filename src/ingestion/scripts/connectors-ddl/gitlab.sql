CREATE DATABASE IF NOT EXISTS `bronze_gitlab`;

CREATE TABLE IF NOT EXISTS bronze_gitlab.branches
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
    `project_id` Nullable(Int64),
    `repository` Nullable(String),
    `repo_path` Nullable(String),
    `name` Nullable(String),
    `head_sha` Nullable(String),
    `head_committed_date` Nullable(String),
    `is_default` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.commit_authors
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
    `project_id` Nullable(Int64),
    `repo_path` Nullable(String),
    `author_email` Nullable(String),
    `author_account_id` Nullable(Int64),
    `author_username` Nullable(String),
    `author_name` Nullable(String),
    `author_state` Nullable(String),
    `matched_field` Nullable(String),
    `sample_sha` Nullable(String),
    `last_committed_date` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.commits
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
    `project_id` Nullable(Int64),
    `repository` Nullable(String),
    `repo_path` Nullable(String),
    `sha` Nullable(String),
    `message` Nullable(String),
    `authored_date` Nullable(String),
    `committed_date` Nullable(String),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `author_account_id` Nullable(Int64),
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

CREATE TABLE IF NOT EXISTS bronze_gitlab.deployments
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
    `project_id` Nullable(Int64),
    `repo_path` Nullable(String),
    `id` Nullable(Int64),
    `iid` Nullable(Int64),
    `ref` Nullable(String),
    `sha` Nullable(String),
    `status` Nullable(String),
    `environment_id` Nullable(Int64),
    `environment_name` Nullable(String),
    `deployable_id` Nullable(Int64),
    `deployable_name` Nullable(String),
    `deployable_stage` Nullable(String),
    `pipeline_id` Nullable(Int64),
    `user_id` Nullable(Int64),
    `user_username` Nullable(String),
    `user_name` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.environments
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
    `project_id` Nullable(Int64),
    `repo_path` Nullable(String),
    `id` Nullable(Int64),
    `name` Nullable(String),
    `slug` Nullable(String),
    `state` Nullable(String),
    `tier` Nullable(String),
    `external_url` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.file_changes
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
    `project_id` Nullable(Int64),
    `repository` Nullable(String),
    `repo_path` Nullable(String),
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

CREATE TABLE IF NOT EXISTS bronze_gitlab.group_members
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
    `scope` Nullable(String),
    `id` Nullable(Int64),
    `username` Nullable(String),
    `name` Nullable(String),
    `state` Nullable(String),
    `access_level` Nullable(Int64),
    `email` Nullable(String),
    `public_email` Nullable(String),
    `membership_state` Nullable(String),
    `created_at` Nullable(String),
    `expires_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.pipelines
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
    `project_id` Nullable(Int64),
    `repo_path` Nullable(String),
    `id` Nullable(Int64),
    `iid` Nullable(Int64),
    `sha` Nullable(String),
    `ref` Nullable(String),
    `status` Nullable(String),
    `source` Nullable(String),
    `user_id` Nullable(Int64),
    `user_username` Nullable(String),
    `user_name` Nullable(String),
    `mr_iid` Nullable(Int64),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `started_at` Nullable(String),
    `finished_at` Nullable(String),
    `duration` Nullable(Int64),
    `queued_duration` Nullable(Decimal(38, 9)),
    `retryable` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_request_commits
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
    `project_id` Nullable(Int64),
    `mr_iid` Nullable(Int64),
    `mr_updated_at` Nullable(String),
    `sha` Nullable(String),
    `short_id` Nullable(String),
    `title` Nullable(String),
    `message` Nullable(String),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `author_account_id` Nullable(Int64),
    `authored_date` Nullable(String),
    `committer_name` Nullable(String),
    `committer_email` Nullable(String),
    `committed_date` Nullable(String),
    `parent_ids` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_request_diff_stats
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
    `project_id` Nullable(Int64),
    `repo_path` Nullable(String),
    `mr_iid` Nullable(Int64),
    `updated_at` Nullable(String),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `files_changed` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_request_label_events
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
    `project_id` Nullable(Int64),
    `mr_iid` Nullable(Int64),
    `id` Nullable(Int64),
    `action` Nullable(String),
    `label_id` Nullable(Int64),
    `label_name` Nullable(String),
    `user_id` Nullable(Int64),
    `user_username` Nullable(String),
    `user_name` Nullable(String),
    `created_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_request_notes
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
    `project_id` Nullable(Int64),
    `mr_iid` Nullable(Int64),
    `mr_updated_at` Nullable(String),
    `id` Nullable(Int64),
    `body` Nullable(String),
    `type` Nullable(String),
    `system` Nullable(Bool),
    `internal` Nullable(Bool),
    `resolvable` Nullable(Bool),
    `resolved` Nullable(Bool),
    `resolved_by_id` Nullable(Int64),
    `author_id` Nullable(Int64),
    `author_username` Nullable(String),
    `author_name` Nullable(String),
    `position_new_path` Nullable(String),
    `position_old_path` Nullable(String),
    `position_new_line` Nullable(Int64),
    `position_old_line` Nullable(Int64),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_request_state_events
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
    `project_id` Nullable(Int64),
    `mr_iid` Nullable(Int64),
    `id` Nullable(Int64),
    `state` Nullable(String),
    `user_id` Nullable(Int64),
    `user_username` Nullable(String),
    `user_name` Nullable(String),
    `created_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.pull_requests
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
    `iid` Nullable(Int64),
    `project_id` Nullable(Int64),
    `title` Nullable(String),
    `description` Nullable(String),
    `state` Nullable(String),
    `draft` Nullable(Bool),
    `author_id` Nullable(Int64),
    `author_username` Nullable(String),
    `author_name` Nullable(String),
    `merged_by_id` Nullable(Int64),
    `merged_by_username` Nullable(String),
    `closed_by_id` Nullable(Int64),
    `closed_by_username` Nullable(String),
    `assignee_ids` Nullable(String),
    `reviewers` Nullable(String),
    `labels` Nullable(String),
    `milestone_id` Nullable(Int64),
    `milestone_title` Nullable(String),
    `source_branch` Nullable(String),
    `target_branch` Nullable(String),
    `source_project_id` Nullable(Int64),
    `target_project_id` Nullable(Int64),
    `sha` Nullable(String),
    `merge_commit_sha` Nullable(String),
    `squash_commit_sha` Nullable(String),
    `squash` Nullable(Bool),
    `detailed_merge_status` Nullable(String),
    `has_conflicts` Nullable(Bool),
    `changes_count` Nullable(String),
    `user_notes_count` Nullable(Int64),
    `upvotes` Nullable(Int64),
    `downvotes` Nullable(Int64),
    `web_url` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `merged_at` Nullable(String),
    `closed_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.repositories
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
    `name` Nullable(String),
    `path` Nullable(String),
    `path_with_namespace` Nullable(String),
    `namespace_id` Nullable(Int64),
    `namespace_full_path` Nullable(String),
    `namespace_kind` Nullable(String),
    `description` Nullable(String),
    `default_branch` Nullable(String),
    `visibility` Nullable(String),
    `archived` Nullable(Bool),
    `empty_repo` Nullable(Bool),
    `issues_enabled` Nullable(Bool),
    `wiki_enabled` Nullable(Bool),
    `is_fork` Nullable(Bool),
    `forked_from_project_path` Nullable(String),
    `mirror` Nullable(Bool),
    `topics` Nullable(String),
    `star_count` Nullable(Int64),
    `forks_count` Nullable(Int64),
    `open_issues_count` Nullable(Int64),
    `repository_size` Nullable(Int64),
    `http_url_to_repo` Nullable(String),
    `web_url` Nullable(String),
    `created_at` Nullable(String),
    `last_activity_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.users
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
    `username` Nullable(String),
    `name` Nullable(String),
    `state` Nullable(String),
    `email` Nullable(String),
    `public_email` Nullable(String),
    `commit_email` Nullable(String),
    `bot` Nullable(Bool),
    `external` Nullable(Bool),
    `is_admin` Nullable(Bool),
    `created_at` Nullable(String),
    `last_activity_on` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

