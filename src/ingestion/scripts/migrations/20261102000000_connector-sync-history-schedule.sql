-- Spec: docs/components/backend/analytics/specs/connector-health.

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS schedules Array(String) DEFAULT [];

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS schedule_timezone LowCardinality(String) DEFAULT '';

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS schedule_origin LowCardinality(String) DEFAULT '';
