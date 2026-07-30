-- Align pre-#517 tables (`persons`, `org_chart`) to the schema +
-- naming conventions in DESIGN §3.8.
ALTER TABLE persons
    MODIFY COLUMN reason TEXT NULL,
    MODIFY COLUMN created_at DATETIME(6) NOT NULL DEFAULT (UTC_TIMESTAMP(6));

ALTER TABLE org_chart
    MODIFY COLUMN reason VARCHAR(50) NULL,
    MODIFY COLUMN valid_from DATETIME(6) NOT NULL,
    MODIFY COLUMN valid_to DATETIME(6) NULL;
