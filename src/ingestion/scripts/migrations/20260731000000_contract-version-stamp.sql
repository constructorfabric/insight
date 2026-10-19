-- Contract version stamp: the deployed version of the read-only surface the
-- Engineering layer exposes to presentation. The analytics service pins an
-- expected version and verifies this stamp periodically.
--
-- On a surface change, bump it HERE (in place, not a new migration file) and
-- in PINNED_CONTRACT_VERSION, src/backend/services/analytics/src/domain/
-- contract_version.rs. Those two are the constant: the stamp and the reader
-- that checks it. A third copy lives in scripts/connectors-ddl/silver.sql and
-- must stay in step while it exists — #3562 deletes that file, and nothing
-- replaces that copy, because the snapshot only pre-created what this
-- migration creates anyway.
--
-- The view needs no engine and no cluster clause: a view holds no rows, and
-- the database it is created in replays its DDL on every replica.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
CREATE OR REPLACE VIEW silver.contract_version AS
SELECT toUInt32(1) AS version;
