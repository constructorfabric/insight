CREATE DATABASE IF NOT EXISTS insight;
-- #1964 writable namespace; role + user provisioned by the seed/migrate path.
CREATE DATABASE IF NOT EXISTS presentation;
-- #2573 adoption events; the grant below needs it to exist before the role runs.
CREATE DATABASE IF NOT EXISTS product_usage;
