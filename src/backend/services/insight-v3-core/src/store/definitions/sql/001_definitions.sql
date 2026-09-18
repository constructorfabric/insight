-- Metric, widget and dashboard definitions.
--
-- One table per kind, each keyed by the name the reader and the assistant use
-- to refer to it. The key is what makes a change an upsert and what stops two
-- writers leaving two rows under one name.

CREATE TABLE IF NOT EXISTS metrics (
    name VARCHAR(128) NOT NULL PRIMARY KEY,
    body JSON NOT NULL,
    updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS widgets (
    name VARCHAR(128) NOT NULL PRIMARY KEY,
    body JSON NOT NULL,
    updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS dashboards (
    name VARCHAR(128) NOT NULL PRIMARY KEY,
    body JSON NOT NULL,
    updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
