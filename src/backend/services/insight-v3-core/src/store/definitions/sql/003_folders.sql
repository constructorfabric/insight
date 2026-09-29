CREATE TABLE IF NOT EXISTS folders (
    id CHAR(36) NOT NULL PRIMARY KEY,
    name VARCHAR(64) NOT NULL COLLATE utf8mb4_uca1400_as_ci,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    UNIQUE KEY folders_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

ALTER TABLE dashboards
    ADD COLUMN IF NOT EXISTS folder_id CHAR(36) NULL,
    ADD CONSTRAINT dashboards_folder FOREIGN KEY IF NOT EXISTS (folder_id)
        REFERENCES folders (id) ON DELETE SET NULL;
