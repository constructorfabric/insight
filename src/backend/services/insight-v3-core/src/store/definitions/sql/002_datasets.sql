CREATE TABLE IF NOT EXISTS datasets (
    name VARCHAR(128) NOT NULL PRIMARY KEY,
    body JSON NOT NULL,
    state VARCHAR(16) NOT NULL,
    physical_table VARCHAR(128) NULL,
    operation VARCHAR(16) NULL,
    operation_token VARCHAR(64) NULL,
    lease_until DATETIME(6) NULL,
    updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
