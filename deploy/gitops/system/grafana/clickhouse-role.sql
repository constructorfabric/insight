-- grafana_ro: read-only role for Grafana's ClickHouse datasource, which charts
-- warehouse load volume from `_airbyte_extracted_at`. Idempotent. Needs an
-- admin with access_management. The user that carries this role is created by
-- provision-clickhouse-user.sh (needs a password → not static DDL).

CREATE ROLE IF NOT EXISTS grafana_ro;

-- SELECT on everything rather than an enumerated contract list: every
-- connector onboarding creates a new `bronze_<name>` database, and a fixed
-- list would silently drop the new connector out of the dashboards until
-- someone noticed. Read-only-by-construction still holds — SELECT and SHOW are
-- the only privileges here, so the wildcard only discloses data the dashboards
-- exist to display.
GRANT SELECT ON *.* TO grafana_ro;
GRANT SHOW   ON *.* TO grafana_ro;
