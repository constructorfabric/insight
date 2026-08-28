-- grafana_ro: SELECT-only role for the Grafana ClickHouse datasource (#2888).
-- Strictly read-only — unlike presentation_ro it takes no INSERT/CREATE
-- anywhere: Grafana only ever runs dashboard queries. Idempotent. Needs an
-- admin with access_management (compose/e2e/bitnami already have it; see
-- README.md). The grant-less `grafana` user that carries this role is created
-- by provision-grafana-access.sh (needs a password -> not static DDL).

CREATE ROLE IF NOT EXISTS grafana_ro;

GRANT SELECT ON silver.* TO grafana_ro;
GRANT SELECT ON identity.* TO grafana_ro;
GRANT SELECT ON insight.* TO grafana_ro;
GRANT SELECT ON presentation.* TO grafana_ro;
GRANT SELECT ON product_usage.* TO grafana_ro;
GRANT SELECT ON ingestion_history.* TO grafana_ro;
