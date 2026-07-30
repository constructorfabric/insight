-- Grant-less `presentation` user analytics connects as (#1964). Local dev
-- password; prod seals clickhouse-presentation-password. Runs after
-- 01-presentation-role.sql (the role it carries).
CREATE USER IF NOT EXISTS presentation IDENTIFIED BY 'presentation-local';
GRANT presentation_ro TO presentation;
ALTER USER presentation DEFAULT ROLE presentation_ro;
