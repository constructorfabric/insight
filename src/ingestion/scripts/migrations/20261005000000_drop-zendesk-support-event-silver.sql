-- Drop silver.zendesk__support_event, the event-grain Zendesk audit fact that
-- the connector's dbt models materialised in `silver` before it moved to
-- `staging` (it is an intermediate of the zendesk connector, not a class
-- contract). No model, view or service reads the silver relation any more;
-- the data is derived and is rebuilt from bronze_zendesk on the next sync.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
DROP TABLE IF EXISTS silver.zendesk__support_event;
