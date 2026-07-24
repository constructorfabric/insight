CREATE DATABASE IF NOT EXISTS `bronze_bamboohr`;

CREATE TABLE IF NOT EXISTS bronze_bamboohr.employees
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `id` Nullable(String),
    `city` Nullable(String),
    `status` Nullable(String),
    `country` Nullable(String),
    `division` Nullable(String),
    `hireDate` Nullable(String),
    `jobTitle` Nullable(String),
    `lastName` Nullable(String),
    `location` Nullable(String),
    `raw_data` Nullable(String),
    `firstName` Nullable(String),
    `source_id` Nullable(String),
    `tenant_id` Nullable(String),
    `workEmail` Nullable(String),
    `department` Nullable(String),
    `supervisor` Nullable(String),
    `unique_key` Nullable(String),
    `displayName` Nullable(String),
    `lastChanged` Nullable(String),
    `supervisorEId` Nullable(String),
    `employeeNumber` Nullable(String),
    `supervisorEmail` Nullable(String),
    `terminationDate` Nullable(String),
    `originalHireDate` Nullable(String),
    `employmentHistoryStatus` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bamboohr.leave_requests
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `type` Nullable(String),
    `id` Nullable(String),
    `end` Nullable(String),
    `name` Nullable(String),
    `dates` Nullable(String),
    `notes` Nullable(String),
    `start` Nullable(String),
    `amount` Nullable(String),
    `status` Nullable(String),
    `actions` Nullable(String),
    `created` Nullable(String),
    `source_id` Nullable(String),
    `tenant_id` Nullable(String),
    `employeeId` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_bamboohr.meta_fields
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `type` Nullable(String),
    `id` Nullable(String),
    `name` Nullable(String),
    `alias` Nullable(String),
    `unique` Nullable(String),
    `source_id` Nullable(String),
    `tenant_id` Nullable(String),
    `deprecated` Nullable(Bool),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

