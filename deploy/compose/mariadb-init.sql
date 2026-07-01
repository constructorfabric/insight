-- Bootstrap databases + users for the local docker-compose dev stack.
-- Loaded by mariadb image on first start (only when the data dir is empty).

CREATE DATABASE IF NOT EXISTS identity  CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE IF NOT EXISTS analytics CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;

CREATE USER IF NOT EXISTS 'insight'@'%' IDENTIFIED BY 'insight-local';
GRANT ALL PRIVILEGES ON identity.*  TO 'insight'@'%';
GRANT ALL PRIVILEGES ON analytics.* TO 'insight'@'%';
FLUSH PRIVILEGES;
