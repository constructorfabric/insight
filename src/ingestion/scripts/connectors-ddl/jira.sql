CREATE DATABASE IF NOT EXISTS `bronze_jira`;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_boards
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `type` Nullable(String),
    `id` Nullable(Decimal(38, 9)),
    `self` Nullable(String),
    `name` Nullable(String),
    `location` Nullable(String),
    `isPrivate` Nullable(Bool)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_comments
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `self` Nullable(String),
    `id` Nullable(String),
    `author` Nullable(String),
    `body` Nullable(String),
    `updateAuthor` Nullable(String),
    `created` Nullable(String),
    `updated` Nullable(String),
    `jsdPublic` Nullable(Bool),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `comment_id` Nullable(Decimal(38, 9)),
    `id_readable` Nullable(String),
    `author_account_id` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_fields
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `key` Nullable(String),
    `name` Nullable(String),
    `untranslatedName` Nullable(String),
    `custom` Nullable(Bool),
    `orderable` Nullable(Bool),
    `navigable` Nullable(Bool),
    `searchable` Nullable(Bool),
    `clauseNames` Nullable(String),
    `schema` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `field_id` Nullable(String),
    `schema_type` Nullable(String),
    `schema_items` Nullable(String),
    `schema_custom` Nullable(String),
    `collected_at` Nullable(String),
    `scope` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `expand` Nullable(String),
    `id` Nullable(String),
    `self` Nullable(String),
    `key` Nullable(String),
    `fields` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `jira_id` Nullable(String),
    `id_readable` Nullable(String),
    `project_key` Nullable(String),
    `status_id` Nullable(Decimal(38, 9)),
    `priority_id` Nullable(Decimal(38, 9)),
    `issuetype_id` Nullable(Decimal(38, 9)),
    `resolution_id` Nullable(Decimal(38, 9)),
    `assignee_id` Nullable(String),
    `reporter_id` Nullable(String),
    `parent_id` Nullable(String),
    `labels_csv` Nullable(String),
    `due_date` Nullable(String),
    `created` Nullable(String),
    `updated` Nullable(String),
    `custom_fields_json` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue_history
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `author` Nullable(String),
    `created` Nullable(String),
    `items` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `id_readable` Nullable(String),
    `author_account_id` Nullable(String),
    `changelog_id` Nullable(Decimal(38, 9)),
    `created_at` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_issue_keys
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `key` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `id_readable` Nullable(String),
    `jira_id` Nullable(String),
    `updated` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_issuetypes
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `description` Nullable(String),
    `self` Nullable(String),
    `id` Nullable(String),
    `iconUrl` Nullable(String),
    `name` Nullable(String),
    `untranslatedName` Nullable(String),
    `subtask` Nullable(Bool),
    `avatarId` Nullable(Decimal(38, 9)),
    `hierarchyLevel` Nullable(Decimal(38, 9)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `issuetype_id` Nullable(Decimal(38, 9)),
    `hierarchy_level` Nullable(Decimal(38, 9)),
    `collected_at` Nullable(String),
    `scope` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_priorities
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `description` Nullable(String),
    `self` Nullable(String),
    `statusColor` Nullable(String),
    `iconUrl` Nullable(String),
    `name` Nullable(String),
    `id` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `priority_id` Nullable(Decimal(38, 9)),
    `collected_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_projects
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `expand` Nullable(String),
    `self` Nullable(String),
    `id` Nullable(String),
    `key` Nullable(String),
    `name` Nullable(String),
    `avatarUrls` Nullable(String),
    `projectCategory` Nullable(String),
    `projectTypeKey` Nullable(String),
    `simplified` Nullable(Bool),
    `style` Nullable(String),
    `isPrivate` Nullable(Bool),
    `properties` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `project_id` Nullable(Decimal(38, 9)),
    `project_key` Nullable(String),
    `project_type` Nullable(String),
    `archived` Nullable(Bool),
    `collected_at` Nullable(String),
    `entityId` Nullable(String),
    `uuid` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_resolutions
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `description` Nullable(String),
    `self` Nullable(String),
    `id` Nullable(String),
    `name` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `resolution_id` Nullable(Decimal(38, 9)),
    `collected_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_sprints
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(Decimal(38, 9)),
    `self` Nullable(String),
    `state` Nullable(String),
    `name` Nullable(String),
    `startDate` Nullable(String),
    `endDate` Nullable(String),
    `completeDate` Nullable(String),
    `originBoardId` Nullable(Decimal(38, 9)),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `sprint_id` Nullable(Decimal(38, 9)),
    `board_id` Nullable(Decimal(38, 9)),
    `sprint_name` Nullable(String),
    `start_date` Nullable(String),
    `end_date` Nullable(String),
    `complete_date` Nullable(String),
    `collected_at` Nullable(String),
    `goal` Nullable(String),
    `createdDate` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_statuses
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `description` Nullable(String),
    `self` Nullable(String),
    `iconUrl` Nullable(String),
    `name` Nullable(String),
    `untranslatedName` Nullable(String),
    `id` Nullable(String),
    `statusCategory` Nullable(String),
    `scope` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `status_id` Nullable(Decimal(38, 9)),
    `category_id` Nullable(Decimal(38, 9)),
    `category_name` Nullable(String),
    `category_key` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = MergeTree
ORDER BY _airbyte_raw_id
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_user
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `self` Nullable(String),
    `accountId` Nullable(String),
    `accountType` Nullable(String),
    `emailAddress` Nullable(String),
    `avatarUrls` Nullable(String),
    `displayName` Nullable(String),
    `active` Nullable(Bool),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `account_id` Nullable(String),
    `email` Nullable(String),
    `display_name` Nullable(String),
    `account_type` Nullable(String),
    `collected_at` Nullable(String),
    `locale` Nullable(String),
    `timeZone` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_jira.jira_worklogs
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `self` Nullable(String),
    `author` Nullable(String),
    `updateAuthor` Nullable(String),
    `created` Nullable(String),
    `updated` Nullable(String),
    `started` Nullable(String),
    `timeSpent` Nullable(String),
    `timeSpentSeconds` Nullable(Decimal(38, 9)),
    `id` Nullable(String),
    `issueId` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` Nullable(String),
    `worklog_id` Nullable(Decimal(38, 9)),
    `id_readable` Nullable(String),
    `author_account_id` Nullable(String),
    `time_spent_seconds` Nullable(Decimal(38, 9)),
    `comment` Nullable(String),
    `collected_at` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

