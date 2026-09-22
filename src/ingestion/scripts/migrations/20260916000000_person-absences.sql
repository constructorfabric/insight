CREATE TABLE IF NOT EXISTS silver.class_person_absences
(
    insight_tenant_id String,
    unique_key String,
    account_source_type String,
    account_source_id String,
    account_id String,
    start_date Date,
    end_date Date
)
ENGINE = ReplacingMergeTree
ORDER BY unique_key;
