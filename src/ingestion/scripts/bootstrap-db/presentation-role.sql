-- presentation_ro: read-only-by-construction role for the presentation query
-- path (#1963). Contract = SELECT only, apart from the semantic measure cache
-- relations the query path materializes itself; `presentation` =
-- SELECT/INSERT/CREATE; `product_usage` = SELECT/INSERT only,
-- `ingestion_history` = SELECT only, both with DDL owned by a migration;
-- no DROP/TRUNCATE anywhere, and no ALTER outside those two named tables.
-- Idempotent. Needs an admin with
-- access_management (compose/e2e/bitnami already have it; see README.md).
-- Spec: docs/domain/presentation-layer/specs.
-- The grant-less `presentation` user that carries this role is created by
-- provision-presentation-access.sh (#1964; needs a password → not static DDL).

CREATE ROLE IF NOT EXISTS presentation_ro;

-- Contract (read-only): silver + identity + legacy gold in `insight`.
GRANT SELECT ON silver.* TO presentation_ro;
GRANT SELECT ON identity.* TO presentation_ro;
GRANT SELECT ON insight.* TO presentation_ro;

-- presentation (writable): no destructive DDL.
GRANT SELECT, INSERT, CREATE ON presentation.* TO presentation_ro;

-- product_usage (append-only): adoption events (#2573). No CREATE — the table
-- comes from migrations/20260816000000_usage-events.sql. SELECT serves the
-- admin-gated usage summary; analytics checks the admin role itself for reads
-- of this database that arrive by any other route.
GRANT SELECT, INSERT ON product_usage.* TO presentation_ro;

-- ingestion_history (read-only): connector sync history. The writer is the
-- reconcile loop, which authenticates as the ingestion admin, so this role
-- takes SELECT and nothing else — not INSERT, not CREATE. Its DDL comes from
-- migrations/20260827000000_connector-sync-history.sql. The grant lands before
-- the database exists on a fresh install (apply-ch-migrations.sh provisions the
-- role first); ClickHouse grants by name, so that ordering is fine.
GRANT SELECT ON ingestion_history.* TO presentation_ro;

-- Semantic measure cache: the only relations in `insight` this role may write.
-- The refresher stages a build, swaps it in one partition at a time and clears
-- the slot around it, so it needs INSERT plus ALTER DELETE on both — ClickHouse
-- checks DROP PARTITION as ALTER DELETE, and REPLACE PARTITION as INSERT +
-- ALTER DELETE on the target with SELECT on the source. Table-scoped, so the
-- database stays read-only everywhere else. Their DDL comes from
-- migrations/20260828000000_semantic-measure-cache.sql; ClickHouse grants by
-- name, so the role may be provisioned before the tables exist.
GRANT INSERT, ALTER DELETE ON insight.semantic_measure_cache TO presentation_ro;
GRANT INSERT, ALTER DELETE ON insight.semantic_measure_cache_staging TO presentation_ro;
