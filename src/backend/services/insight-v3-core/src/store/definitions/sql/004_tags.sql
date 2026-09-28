CREATE TABLE IF NOT EXISTS tags (
    id CHAR(36) NOT NULL PRIMARY KEY,
    name VARCHAR(32) NOT NULL COLLATE utf8mb4_uca1400_as_ci,
    UNIQUE KEY tags_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS dashboard_tags (
    dashboard VARCHAR(128) NOT NULL,
    tag_id CHAR(36) NOT NULL,
    PRIMARY KEY (dashboard, tag_id),
    KEY dashboard_tags_tag (tag_id),
    CONSTRAINT dashboard_tags_dashboard FOREIGN KEY (dashboard)
        REFERENCES dashboards (name) ON DELETE CASCADE,
    CONSTRAINT dashboard_tags_tag FOREIGN KEY (tag_id)
        REFERENCES tags (id) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS tags_lock (
    id TINYINT NOT NULL PRIMARY KEY
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

INSERT IGNORE INTO tags_lock (id) VALUES (1);
