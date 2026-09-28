-- Alert rules and the notifications their checks decided are owed.
--
-- A rule carries its latest check on the row: what was seen, when, and what
-- the last valid check found. A notification is a copy of the facts a check
-- saw, so an edit to the rule afterwards rewrites nothing it will say.

CREATE TABLE IF NOT EXISTS alert_rules (
    id CHAR(32) NOT NULL PRIMARY KEY,
    name VARCHAR(128) NOT NULL UNIQUE,
    metric VARCHAR(128) NOT NULL,
    column_name VARCHAR(128) NOT NULL,
    operator VARCHAR(2) NOT NULL,
    -- The number as text, so an integer stays exact.
    threshold VARCHAR(64) NOT NULL,
    range_code VARCHAR(32) NULL,
    interval_secs INT UNSIGNED NOT NULL,
    destination VARCHAR(128) NOT NULL,
    enabled TINYINT(1) NOT NULL,
    -- Bumped by every configuration write; a check is recorded only at the
    -- revision it was scheduled for.
    revision INT UNSIGNED NOT NULL,
    last_evaluated_at DATETIME(6) NULL,
    -- breach, no_breach or unknown.
    last_outcome VARCHAR(16) NULL,
    last_reason VARCHAR(32) NULL,
    last_value VARCHAR(64) NULL,
    -- What the last valid check found; unknown checks leave it as it was.
    last_valid_breached TINYINT(1) NULL,
    breached_since DATETIME(6) NULL,
    -- The API resolves who wrote the rule; the MCP server cannot.
    created_by CHAR(32) NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;

CREATE TABLE IF NOT EXISTS alert_notifications (
    id CHAR(32) NOT NULL PRIMARY KEY,
    rule_id CHAR(32) NOT NULL,
    rule_revision INT UNSIGNED NOT NULL,
    rule_name VARCHAR(128) NOT NULL,
    metric VARCHAR(128) NOT NULL,
    column_name VARCHAR(128) NOT NULL,
    operator VARCHAR(2) NOT NULL,
    threshold VARCHAR(64) NOT NULL,
    value VARCHAR(64) NOT NULL,
    evaluated_at DATETIME(6) NOT NULL,
    destination VARCHAR(128) NOT NULL,
    -- pending or cancelled; delivery adds its own words.
    status VARCHAR(16) NOT NULL,
    attempts INT UNSIGNED NOT NULL,
    last_error VARCHAR(1000) NULL,
    provider_receipt VARCHAR(256) NULL,
    created_at DATETIME(6) NOT NULL,
    updated_at DATETIME(6) NOT NULL,
    INDEX alert_notifications_by_rule (rule_id, created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
