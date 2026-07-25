-- Align account_person_map SCD2 columns with the DESIGN §3.8 conventions
-- (DATETIME(6), like persons/org_chart after 009). TIMESTAMP(6) is
-- session-timezone-dependent: the same stored instant reads differently
-- between the .NET connection (server tz) and the Rust identity-resolution
-- pool (pinned time_zone='+00:00'), skewing SCD2 valid_from/valid_to
-- comparisons whenever the MariaDB server tz is not UTC. TIMESTAMP is also
-- 2038-bounded.
--
-- The session tz is pinned to UTC for the conversion: MariaDB converts
-- TIMESTAMP -> DATETIME through the session timezone, so this renders the
-- stored UTC instants as UTC literals regardless of the server default.
SET time_zone = '+00:00';

ALTER TABLE account_person_map
    MODIFY COLUMN valid_from DATETIME(6) NOT NULL,
    MODIFY COLUMN valid_to DATETIME(6) NULL;
