CREATE TABLE IF NOT EXISTS people (
    id BIGINT UNSIGNED AUTO_INCREMENT PRIMARY KEY,
    insight_tenant_id BINARY(16) NOT NULL,
    person_id BINARY(16) NOT NULL,
    email VARCHAR(320) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
    username VARCHAR(320) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
    display_name VARCHAR(512) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
    first_name VARCHAR(512) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
    last_name VARCHAR(512) CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci NULL,
    attributes JSON NOT NULL,
    valid_from DATETIME(6) NOT NULL,
    valid_to DATETIME(6) NULL,
    current_person_id BINARY(16)
        GENERATED ALWAYS AS (CASE WHEN valid_to IS NULL THEN person_id ELSE NULL END) STORED,
    UNIQUE KEY uq_people_current (insight_tenant_id, current_person_id),
    INDEX idx_people_current (insight_tenant_id, valid_to, person_id),
    CONSTRAINT chk_people_interval CHECK (valid_to IS NULL OR valid_to >= valid_from)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
