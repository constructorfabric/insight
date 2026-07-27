CREATE DATABASE IF NOT EXISTS `bronze_workday`;

CREATE TABLE IF NOT EXISTS bronze_workday.leave_requests
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `Request_ID` Nullable(String),
    `Employee_ID` Nullable(String),
    `Time_Off_Type` Nullable(String),
    `Start_Date` Nullable(String),
    `End_Date` Nullable(String),
    `Quantity` Nullable(String),
    `Unit` Nullable(String),
    `Status` Nullable(String),
    `Submitted_Moment` Nullable(String),
    `raw_data` Nullable(String),
    `source_id` Nullable(String),
    `tenant_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

CREATE TABLE IF NOT EXISTS bronze_workday.workers
(
    `_airbyte_raw_id` String,
    `_airbyte_extracted_at` DateTime64(3),
    `_airbyte_meta` String,
    `_airbyte_generation_id` UInt32,
    `Employee_ID` Nullable(String),
    `Display_Name` Nullable(String),
    `First_Name` Nullable(String),
    `Last_Name` Nullable(String),
    `Work_Email` Nullable(String),
    `Business_Title` Nullable(String),
    `Job_Profile` Nullable(String),
    `Worker_Type` Nullable(String),
    `Worker_Status` Nullable(String),
    `Supervisory_Organization` Nullable(String),
    `Manager_Employee_ID` Nullable(String),
    `Manager_Work_Email` Nullable(String),
    `Location` Nullable(String),
    `Country` Nullable(String),
    `City` Nullable(String),
    `Hire_Date` Nullable(String),
    `Original_Hire_Date` Nullable(String),
    `Termination_Date` Nullable(String),
    `Last_Functionally_Updated` Nullable(String),
    `Scheduled_Weekly_Hours` Nullable(String),
    `raw_data` Nullable(String),
    `source_id` Nullable(String),
    `tenant_id` Nullable(String),
    `unique_key` Nullable(String)
)
ENGINE = ReplacingMergeTree(_airbyte_extracted_at)
ORDER BY unique_key
SETTINGS allow_nullable_key = 1, index_granularity = 8192
;

