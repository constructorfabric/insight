-- Compose-only ClickHouse bootstrap, run once by the server's
-- docker-entrypoint-initdb.d on a fresh volume.
--
-- No CREATE DATABASE here: src/ingestion/scripts/create-databases.sh is the one
-- site that creates a database, and `dev-compose.sh seed` runs it. A ClickHouse
-- grant resolves by name, so the roles below stand without their databases.
--
-- Local dev password. 01-presentation-role.sql carries this role's grants
-- but runs later, and a role grant resolves now, so the role starts here.
CREATE ROLE IF NOT EXISTS insight_v3_ro;
CREATE USER IF NOT EXISTS insight_v3_reader IDENTIFIED BY 'insight-v3-reader-local';
GRANT insight_v3_ro TO insight_v3_reader;
ALTER USER insight_v3_reader DEFAULT ROLE insight_v3_ro;
