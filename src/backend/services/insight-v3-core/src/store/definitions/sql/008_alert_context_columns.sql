-- The result columns a rule asks every notification to carry beside the
-- ones its condition reads, as a JSON list of names. Null is none: a rule
-- or notification written before rules carried any.

ALTER TABLE alert_rules ADD COLUMN IF NOT EXISTS context_columns MEDIUMTEXT NULL AFTER rule_condition;

ALTER TABLE alert_notifications ADD COLUMN IF NOT EXISTS context_columns MEDIUMTEXT NULL AFTER rule_condition;
