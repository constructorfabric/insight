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
    `name` Nullable(String),
    `commit_sha` Nullable(String),
    `default` Nullable(Bool),
    `protected` Nullable(Bool),
    `merged` Nullable(Bool)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.commit_file_changes
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
    `commit_sha` Nullable(String),
    `old_path` Nullable(String),
    `new_path` Nullable(String),
    `new_file` Nullable(Bool),
    `deleted_file` Nullable(Bool),
    `renamed_file` Nullable(Bool),
    `lines_added` Nullable(Int64),
    `lines_removed` Nullable(Int64),
    `diff_truncated` Nullable(Bool)
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
    `id` Nullable(String),
    `short_id` Nullable(String),
    `title` Nullable(String),
    `title_truncated` Nullable(Bool),
    `message` Nullable(String),
    `message_truncated` Nullable(Bool),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `authored_date` Nullable(String),
    `committer_name` Nullable(String),
    `committer_email` Nullable(String),
    `committed_date` Nullable(String),
    `parent_count` Nullable(Int64),
    `stats_additions` Nullable(Int64),
    `stats_deletions` Nullable(Int64),
    `stats_total` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.issues
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
    `iid` Nullable(Int64),
    `id` Nullable(Int64),
    `title` Nullable(String),
    `title_truncated` Nullable(Bool),
    `description` Nullable(String),
    `description_truncated` Nullable(Bool),
    `state` Nullable(String),
    `author_id` Nullable(Int64),
    `author_username` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `closed_at` Nullable(String),
    `closed_by_id` Nullable(Int64),
    `milestone_id` Nullable(Int64),
    `user_notes_count` Nullable(Int64),
    `assignee_ids` Nullable(String),
    `labels` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_request_approvals
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
    `approvals_required` Nullable(Int64),
    `approvals_left` Nullable(Int64),
    `approved` Nullable(Bool),
    `approved_by` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_request_commits
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
    `id` Nullable(String),
    `short_id` Nullable(String),
    `title` Nullable(String),
    `title_truncated` Nullable(Bool),
    `message` Nullable(String),
    `message_truncated` Nullable(Bool),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `authored_date` Nullable(String),
    `committer_name` Nullable(String),
    `committer_email` Nullable(String),
    `committed_date` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_request_discussions
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
    `discussion_id` Nullable(String),
    `individual_note` Nullable(Bool),
    `note_ids` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_request_notes
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
    `body_truncated` Nullable(Bool),
    `author_id` Nullable(Int64),
    `author_username` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `system` Nullable(Bool),
    `resolvable` Nullable(Bool),
    `resolved` Nullable(Bool),
    `resolved_by_id` Nullable(Int64),
    `noteable_type` Nullable(String),
    `position_new_path` Nullable(String),
    `position_new_line` Nullable(Int64)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_request_state_events
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
    `user_id` Nullable(Int64),
    `user_username` Nullable(String),
    `state` Nullable(String),
    `created_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.merge_requests
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
    `iid` Nullable(Int64),
    `id` Nullable(Int64),
    `title` Nullable(String),
    `title_truncated` Nullable(Bool),
    `description` Nullable(String),
    `description_truncated` Nullable(Bool),
    `state` Nullable(String),
    `draft` Nullable(Bool),
    `author_id` Nullable(Int64),
    `author_username` Nullable(String),
    `merged_by_id` Nullable(Int64),
    `merged_by_username` Nullable(String),
    `source_branch` Nullable(String),
    `target_branch` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `merged_at` Nullable(String),
    `closed_at` Nullable(String),
    `sha` Nullable(String),
    `merge_commit_sha` Nullable(String),
    `squash_commit_sha` Nullable(String),
    `squash` Nullable(Bool),
    `merge_status` Nullable(String),
    `user_notes_count` Nullable(Int64),
    `milestone_id` Nullable(Int64),
    `assignee_ids` Nullable(String),
    `reviewer_ids` Nullable(String),
    `labels` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_gitlab.projects
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
    `description` Nullable(String),
    `default_branch` Nullable(String),
    `visibility` Nullable(String),
    `archived` Nullable(Bool),
    `empty_repo` Nullable(Bool),
    `created_at` Nullable(String),
    `last_activity_at` Nullable(String),
    `web_url` Nullable(String),
    `namespace_id` Nullable(Int64),
    `namespace_full_path` Nullable(String),
    `statistics_commit_count` Nullable(Int64),
    `statistics_repository_size` Nullable(Int64)
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
    `bot` Nullable(Bool),
    `web_url` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

