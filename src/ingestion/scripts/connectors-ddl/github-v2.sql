CREATE DATABASE IF NOT EXISTS `bronze_github`;

CREATE TABLE IF NOT EXISTS bronze_github.branches
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `name` Nullable(String),
    `commit` Nullable(String),
    `protected` Nullable(Bool),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String),
    `default_branch_name` Nullable(String),
    `pushed_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.commits
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `sha` Nullable(String),
    `message` Nullable(String),
    `committed_date` Nullable(String),
    `authored_date` Nullable(String),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `changed_files` Nullable(Int64),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `committer_name` Nullable(String),
    `committer_email` Nullable(String),
    `committer_login` Nullable(String),
    `committer_id` Nullable(Int64),
    `parent_hashes` Nullable(String),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String),
    `branch_name` Nullable(String),
    `default_branch_name` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.file_changes
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `source_type` Nullable(String),
    `sha` Nullable(String),
    `filename` Nullable(String),
    `status` Nullable(String),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `changes` Nullable(Int64),
    `previous_filename` Nullable(String),
    `patch` Nullable(String),
    `committed_date` Nullable(String),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `database_id` Nullable(Int64),
    `pr_number` Nullable(Int64),
    `pull_request_id` Nullable(Int64),
    `body` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `author_association` Nullable(String),
    `pull_request_updated_at` Nullable(String),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_commits
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `pull_request_id` Nullable(Int64),
    `pr_number` Nullable(Int64),
    `sha` Nullable(String),
    `committed_date` Nullable(String),
    `pull_request_updated_at` Nullable(String),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_review_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `database_id` Nullable(Int64),
    `pr_number` Nullable(Int64),
    `pull_request_id` Nullable(Int64),
    `body` Nullable(String),
    `filename` Nullable(String),
    `line` Nullable(Int64),
    `start_line` Nullable(Int64),
    `diff_hunk` Nullable(String),
    `commit_id` Nullable(String),
    `original_commit_id` Nullable(String),
    `in_reply_to_id` Nullable(Int64),
    `thread_resolved` Nullable(Bool),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `author_association` Nullable(String),
    `pull_request_updated_at` Nullable(String),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_request_reviews
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `database_id` Nullable(Int64),
    `pr_number` Nullable(Int64),
    `pull_request_id` Nullable(Int64),
    `state` Nullable(String),
    `body` Nullable(String),
    `submitted_at` Nullable(String),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `author_association` Nullable(String),
    `commit_id` Nullable(String),
    `pull_request_updated_at` Nullable(String),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.pull_requests
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `database_id` Nullable(Int64),
    `number` Nullable(Int64),
    `title` Nullable(String),
    `body` Nullable(String),
    `state` Nullable(String),
    `is_draft` Nullable(Bool),
    `review_decision` Nullable(String),
    `labels` Nullable(String),
    `milestone_title` Nullable(String),
    `merge_commit_sha` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `closed_at` Nullable(String),
    `merged_at` Nullable(String),
    `head_ref` Nullable(String),
    `base_ref` Nullable(String),
    `additions` Nullable(Int64),
    `deletions` Nullable(Int64),
    `changed_files` Nullable(Int64),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `author_email` Nullable(String),
    `merged_by_login` Nullable(String),
    `merged_by_id` Nullable(Int64),
    `commit_count` Nullable(Int64),
    `comment_count` Nullable(Int64),
    `review_count` Nullable(Int64),
    `requested_reviewers` Nullable(String),
    `requested_teams` Nullable(String),
    `repo_owner` Nullable(String),
    `repo_name` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_github.repositories
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `data_source` Nullable(String),
    `collected_at` Nullable(String),
    `repo_owner` Nullable(String),
    `name` Nullable(String),
    `full_name` Nullable(String),
    `private` Nullable(Bool),
    `description` Nullable(String),
    `language` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `pushed_at` Nullable(String),
    `size` Nullable(Int64),
    `default_branch` Nullable(String),
    `has_issues` Nullable(Bool),
    `has_wiki` Nullable(Bool),
    `fork` Nullable(Bool),
    `archived` Nullable(Bool)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

