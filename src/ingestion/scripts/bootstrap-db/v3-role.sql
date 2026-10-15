-- insight_v3_ro: the custom assistant's query path (#3558). SELECT and nothing
-- else; the grant-less `insight_v3_reader` user that carries it is created by
-- provision-v3-access.sh (needs a password -> not static DDL).
--
-- `*.*` stays, and the role is gated instead. The alternative was enumerating
-- databases at deploy time, which this grant cannot do and stay correct: an
-- assistant metric may name any warehouse table (#3524), and a stand gains a
-- bronze database whenever a connector is added, so a scoped grant would go
-- stale between deploys and fail a query that was valid when it was written.
-- What an installation gets instead is an off switch — create-databases.sh
-- provisions this only where insight-v3-core is deployed, the way the MCP
-- explorer's narrower role is already gated. An installation that never runs
-- the assistant carries no role that can read every table.

CREATE ROLE IF NOT EXISTS insight_v3_ro;

-- Covers the bronze databases this file cannot enumerate and system.columns,
-- which the catalogue reads.
GRANT SELECT ON *.* TO insight_v3_ro;
