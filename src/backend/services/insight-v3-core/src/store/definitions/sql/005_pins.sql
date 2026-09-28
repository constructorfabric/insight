CREATE TABLE IF NOT EXISTS dashboard_pins (
    person CHAR(36) NOT NULL,
    dashboard VARCHAR(128) NOT NULL,
    pinned_at DATETIME(6) NOT NULL,
    PRIMARY KEY (person, dashboard),
    KEY dashboard_pins_dashboard (dashboard),
    CONSTRAINT dashboard_pins_dashboard FOREIGN KEY (dashboard)
        REFERENCES dashboards (name) ON DELETE CASCADE
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS pins_lock (
    id TINYINT NOT NULL PRIMARY KEY
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

INSERT IGNORE INTO pins_lock (id) VALUES (1);
