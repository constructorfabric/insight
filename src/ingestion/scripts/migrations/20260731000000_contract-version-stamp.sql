-- Contract version stamp: the deployed version of the read-only surface the
-- Engineering layer exposes to presentation. The analytics service pins an
-- expected version and verifies this stamp periodically.
--
-- On a surface change, bump it HERE (in place, not a new migration file) and
-- in PINNED_CONTRACT_VERSION, src/backend/services/analytics/src/domain/
-- contract_version.rs. Those two are the constant: the stamp and the reader
-- that checks it. There are only those two: the DDL snapshot held a third copy
-- until #3562 deleted it, and nothing replaced it, because the snapshot only
-- pre-created what this migration creates anyway.
--
-- The view needs no engine and no cluster clause: a view holds no rows, and
-- the database it is created in replays its DDL on every replica.
--
-- Idempotent: this channel has no ledger and re-runs on every deploy.
CREATE OR REPLACE VIEW silver.contract_version AS
SELECT toUInt32(1) AS version;
