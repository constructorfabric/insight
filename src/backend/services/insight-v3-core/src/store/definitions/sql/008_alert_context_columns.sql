ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS context_columns MEDIUMTEXT NULL AFTER rule_condition;

ALTER TABLE alert_notifications ADD COLUMN IF NOT EXISTS context_columns MEDIUMTEXT NULL AFTER rule_condition;
