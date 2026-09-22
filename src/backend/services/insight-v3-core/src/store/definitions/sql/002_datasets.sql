-- Datasets: the declaration, and where its records are kept.
--
-- Unlike the other kinds a dataset owns a table in the warehouse, so its row
-- carries more than a body: which state it is in, which table holds its
-- records, and which attempt, if any, currently owns it.
--
-- The name is the key, so taking a name is an insert that either succeeds or
-- tells the second writer the name is held.

CREATE TABLE IF NOT EXISTS datasets (
    name VARCHAR(128) NOT NULL PRIMARY KEY,
    body JSON NOT NULL,
    -- claimed, ready or removing; a dataset with no row is absent.
    state VARCHAR(16) NOT NULL,
    -- The table this dataset's records are in, named for the attempt that
    -- made it. Absent until one is provisioned.
    physical_table VARCHAR(128) NULL,
    -- The operation an attempt holds this dataset for, and who holds it.
    -- All three are set together or not at all.
    operation VARCHAR(16) NULL,
    operation_token VARCHAR(64) NULL,
    lease_until DATETIME(6) NULL,
    updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
