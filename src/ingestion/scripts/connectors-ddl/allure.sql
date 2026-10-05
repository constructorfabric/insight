CREATE DATABASE IF NOT EXISTS `bronze_allure`;

CREATE TABLE IF NOT EXISTS bronze_allure.launches
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(Int64),
    `projectId` Nullable(Int64),
    `name` Nullable(String),
    `closed` Nullable(Bool),
    `autoclose` Nullable(Bool),
    `external` Nullable(Bool),
    `createdBy` Nullable(String),
    `createdDate` Nullable(Int64),
    `lastModifiedBy` Nullable(String),
    `lastModifiedDate` Nullable(Int64),
    `tags` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_allure.projects
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(Int64),
    `name` Nullable(String),
    `abbr` Nullable(String),
    `description` Nullable(String),
    `descriptionHtml` Nullable(String),
    `isPublic` Nullable(Bool),
    `favorite` Nullable(Bool),
    `createdBy` Nullable(String),
    `createdDate` Nullable(Int64),
    `lastModifiedBy` Nullable(String),
    `lastModifiedDate` Nullable(Int64),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_allure.test_results
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(Int64),
    `launchId` Nullable(Int64),
    `projectId` Nullable(Int64),
    `testCaseId` Nullable(Int64),
    `name` Nullable(String),
    `fullName` Nullable(String),
    `status` Nullable(String),
    `duration` Nullable(Int64),
    `start` Nullable(Int64),
    `stop` Nullable(Int64),
    `createdBy` Nullable(String),
    `createdDate` Nullable(Int64),
    `lastModifiedBy` Nullable(String),
    `lastModifiedDate` Nullable(Int64),
    `external` Nullable(Bool),
    `flaky` Nullable(Bool),
    `hidden` Nullable(Bool),
    `known` Nullable(Bool),
    `manual` Nullable(Bool),
    `muted` Nullable(Bool),
    `historyKey` Nullable(String),
    `hostId` Nullable(String),
    `threadId` Nullable(String),
    `message` Nullable(String),
    `trace` Nullable(String),
    `description` Nullable(String),
    `descriptionHtml` Nullable(String),
    `statusTransition` Nullable(String),
    `layer` Nullable(String),
    `jobRun` Nullable(String),
    `tags` Nullable(String),
    `links` Nullable(String),
    `parameters` Nullable(String),
    `tenant_id` Nullable(String),
    `source_id` Nullable(String),
    `unique_key` String
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS index_granularity = 8192
;

