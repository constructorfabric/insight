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

CREATE TABLE IF NOT EXISTS bronze_github.commit_authors
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
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `author_type` Nullable(String),
    `sample_sha` Nullable(String)
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
    `description` Nullable(String),
    `creator_login` Nullable(String),
    `creator_id` Nullable(Int64),
    `target_url` Nullable(String),
    `log_url` Nullable(String),
    `environment_url` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
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
    `original_environment` Nullable(String),
    `is_transient_environment` Nullable(Bool),
    `is_production_environment` Nullable(Bool),
    `description` Nullable(String),
    `creator_login` Nullable(String),
    `creator_id` Nullable(Int64),
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
    `actor_id` Nullable(Int64),
    `target_login` Nullable(String),
    `target_id` Nullable(Int64),
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
    `body` Nullable(String),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `author_type` Nullable(String),
    `author_association` Nullable(String),
    `assignee_logins` Nullable(String),
    `assignee_ids` Nullable(String),
    `label_names` Nullable(String),
    `issue_type` Nullable(String),
    `milestone_title` Nullable(String),
    `milestone_number` Nullable(Int64),
    `closed_by_login` Nullable(String),
    `closed_by_id` Nullable(Int64),
    `locked` Nullable(Bool),
    `reactions_total` Nullable(Int64),
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
    `author_id` Nullable(Int64),
    `author_type` Nullable(String),
    `is_via_github_app` Nullable(Bool),
    `reactions_total` Nullable(Int64)
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
    `repo_full_name` Nullable(String),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `author_type` Nullable(String),
    `author_name` Nullable(String),
    `author_email` Nullable(String),
    `authored_date` Nullable(String),
    `committer_login` Nullable(String),
    `committer_id` Nullable(Int64),
    `committer_name` Nullable(String),
    `committer_email` Nullable(String),
    `committed_date` Nullable(String),
    `message` Nullable(String),
    `parent_shas` Nullable(String),
    `is_merge` Nullable(Bool),
    `is_verified` Nullable(Bool),
    `verification_reason` Nullable(String)
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
    `author_email` Nullable(String),
    `author_login` Nullable(String),
    `author_id` Nullable(Int64),
    `merged_by_login` Nullable(String),
    `merged_by_id` Nullable(Int64),
    `review_decision` Nullable(String),
    `total_comments_count` Nullable(Int64),
    `is_cross_repository` Nullable(Bool)
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
    `author_type` Nullable(String),
    `path` Nullable(String),
    `line` Nullable(Int64),
    `start_line` Nullable(Int64),
    `side` Nullable(String),
    `subject_type` Nullable(String),
    `commit_id` Nullable(String),
    `original_commit_id` Nullable(String),
    `review_id` Nullable(Int64),
    `in_reply_to_id` Nullable(Int64),
    `diff_hunk` Nullable(String),
    `is_via_github_app` Nullable(Bool),
    `reactions_total` Nullable(Int64)
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
    `author_id` Nullable(Int64),
    `author_type` Nullable(String),
    `body` Nullable(String)
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
    `actor_id` Nullable(Int64),
    `target_login` Nullable(String),
    `target_id` Nullable(Int64),
    `label_name` Nullable(String),
    `state_reason` Nullable(String),
    `merge_commit_sha` Nullable(String)
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
    `author_id` Nullable(Int64),
    `author_type` Nullable(String),
    `author_association` Nullable(String),
    `head_sha` Nullable(String),
    `head_ref` Nullable(String),
    `head_label` Nullable(String),
    `head_repo_full_name` Nullable(String),
    `base_ref` Nullable(String),
    `base_sha` Nullable(String),
    `merge_commit_sha` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String),
    `closed_at` Nullable(String),
    `merged_at` Nullable(String),
    `body` Nullable(String),
    `locked` Nullable(Bool),
    `active_lock_reason` Nullable(String),
    `auto_merge_enabled` Nullable(Bool),
    `label_names` Nullable(String),
    `assignee_logins` Nullable(String),
    `requested_reviewer_logins` Nullable(String),
    `requested_team_slugs` Nullable(String),
    `milestone_title` Nullable(String),
    `milestone_number` Nullable(Int64)
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
    `has_projects` Nullable(Bool),
    `has_discussions` Nullable(Bool),
    `has_pages` Nullable(Bool),
    `is_template` Nullable(Bool),
    `disabled` Nullable(Bool),
    `web_commit_signoff_required` Nullable(Bool),
    `visibility` Nullable(String),
    `license_spdx_id` Nullable(String),
    `topics` Nullable(String),
    `homepage` Nullable(String),
    `stargazers_count` Nullable(Int64),
    `forks_count` Nullable(Int64),
    `watchers_count` Nullable(Int64),
    `open_issues_count` Nullable(Int64),
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
    `actor_id` Nullable(Int64),
    `triggering_actor_login` Nullable(String),
    `triggering_actor_id` Nullable(Int64),
    `run_number` Nullable(Int64),
    `workflow_path` Nullable(String),
    `display_title` Nullable(String),
    `check_suite_id` Nullable(Int64),
    `pull_request_numbers` Nullable(String),
    `head_commit_message` Nullable(String),
    `head_commit_timestamp` Nullable(String),
    `head_commit_author_name` Nullable(String),
    `head_commit_author_email` Nullable(String),
    `run_started_at` Nullable(String),
    `created_at` Nullable(String),
    `updated_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

