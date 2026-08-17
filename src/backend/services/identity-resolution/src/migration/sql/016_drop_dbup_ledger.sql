-- The retired applier's own migration ledger. This service tracks its
-- migrations in `seaql_migrations`; the old table was left in place while
-- the previous owner could still be brought back, and has no reader now.
DROP TABLE IF EXISTS SchemaVersions;
