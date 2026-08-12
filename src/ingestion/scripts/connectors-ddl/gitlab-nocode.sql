CREATE DATABASE IF NOT EXISTS `bronze_gitlab_nocode`;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.branches
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `project_id` Nullable(String),
    `name` Nullable(String),
    `head_sha` Nullable(String),
    `head_committed_date` Nullable(String),
    `is_default` Nullable(Bool)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.commits
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `project_id` Nullable(String),
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
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.deployments
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
    `status` Nullable(String),
    `ref` Nullable(String),
    `sha` Nullable(String),
    `environment_name` Nullable(String),
    `deployable_status` Nullable(String),
    `pipeline_id` Nullable(Int64),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.file_changes
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `project_id` Nullable(String),
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
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.merge_request_approvals
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
    `mr_iid` Nullable(Int64),
    `project_id` Nullable(Int64),
    `approved` Nullable(Bool),
    `approvals_required` Nullable(Int64),
    `approvals_left` Nullable(Int64),
    `approved_by_usernames` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.merge_request_notes
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
    `mr_iid` Nullable(Int64),
    `project_id` Nullable(Int64),
    `author_username` Nullable(String),
    `system` Nullable(Bool),
    `type` Nullable(String),
    `resolvable` Nullable(Bool),
    `resolved` Nullable(Bool),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.merge_requests
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
    `state` Nullable(String),
    `draft` Nullable(Bool),
    `title` Nullable(String),
    `author_username` Nullable(String),
    `source_branch` Nullable(String),
    `target_branch` Nullable(String),
    `sha` Nullable(String),
    `merge_commit_sha` Nullable(String),
    `squash_commit_sha` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `merged_at` Nullable(String),
    `closed_at` Nullable(String),
    `merged_by_username` Nullable(String),
    `user_notes_count` Nullable(Int64)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.pipelines
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
    `status` Nullable(String),
    `source` Nullable(String),
    `ref` Nullable(String),
    `sha` Nullable(String),
    `name` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.repositories
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `unique_key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `data_source` Nullable(String),
    `project_id` Nullable(Int64),
    `path_with_namespace` Nullable(String),
    `name` Nullable(String),
    `default_branch` Nullable(String),
    `visibility` Nullable(String),
    `archived` Nullable(Bool),
    `web_url` Nullable(String),
    `http_url_to_repo` Nullable(String),
    `created_at` Nullable(String),
    `last_activity_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab_nocode.users
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
    `access_level` Nullable(Int64),
    `created_at` Nullable(String),
    `group` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;
