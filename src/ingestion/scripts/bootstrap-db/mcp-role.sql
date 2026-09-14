CREATE ROLE IF NOT EXISTS insight_mcp_ro;

GRANT SELECT ON bronze_*.* TO insight_mcp_ro;
GRANT SELECT ON staging.* TO insight_mcp_ro;
GRANT SELECT ON silver.* TO insight_mcp_ro;
GRANT SELECT ON identity.* TO insight_mcp_ro;
GRANT SELECT ON config.* TO insight_mcp_ro;

GRANT SELECT ON system.databases TO insight_mcp_ro;
GRANT SELECT ON system.tables TO insight_mcp_ro;
GRANT SELECT ON system.columns TO insight_mcp_ro;
