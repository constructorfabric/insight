-- Carry the connector instance's identity in the sync ledger.
--
-- One connector can be installed more than once — a second Secret carrying its
-- own source id, reading a different account of the same vendor — and until now
-- every row of both landed under one `connector` name. The page resolves the
-- newest sync per connector, so two instances would resolve to one row and the
-- older instance would disappear behind the newer one's syncs.
--
-- `CREATE TABLE IF NOT EXISTS` never widens a relation that already exists, so
-- the columns arrive here for an install holding history and in the creating
-- migration for a fresh one. `AFTER connector` in both places keeps the two
-- kinds of install physically identical.
--
-- Existing rows carry an empty identity, which the reconcile sweep fills in
-- from the instance each connector actually has — never from a naming
-- convention. The sort key is deliberately unchanged: this is a plain
-- MergeTree, so the key buys read locality rather than identity, and rewriting
-- it on an install holding history buys nothing for it.
--
-- Spec: docs/components/backend/analytics/specs/connector-health.

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS tenant_id LowCardinality(String) AFTER connector;

ALTER TABLE ingestion_history.sync_events
    ADD COLUMN IF NOT EXISTS source_id LowCardinality(String) AFTER tenant_id;
