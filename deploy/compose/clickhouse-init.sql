CREATE DATABASE IF NOT EXISTS insight;
-- #1964 writable namespace; role + user provisioned by the seed/migrate path.
CREATE DATABASE IF NOT EXISTS presentation;
-- #2573 adoption events; the grant below needs it to exist before the role runs.
CREATE DATABASE IF NOT EXISTS product_usage;

-- Local dev password. 01-presentation-role.sql carries this role's grants
-- but runs later, and a role grant resolves now, so the role starts here.
CREATE ROLE IF NOT EXISTS insight_v3_ro;
CREATE USER IF NOT EXISTS insight_v3_reader IDENTIFIED BY 'insight-v3-reader-local';
GRANT insight_v3_ro TO insight_v3_reader;
ALTER USER insight_v3_reader DEFAULT ROLE insight_v3_ro;
